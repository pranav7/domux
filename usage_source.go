package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// UsageWindow is one agent subscription rate-limit window.
type UsageWindow struct {
	Source UsageSource
	Label  string
	// Percent is rounded from the API's raw value and floored at 0, but NOT
	// capped at 100 — the API can genuinely report overage (e.g. 102%), and
	// silently flooring that to 100 would misrepresent real usage.
	Percent  int
	ResetsAt time.Time // zero if the endpoint omitted / sent an unparseable time
}

// UsageSource identifies the agent that owns a subscription window.
type UsageSource string

const (
	usageClaude UsageSource = "claude"
	usageCodex  UsageSource = "codex"
)

// UsageSnapshot is the normalized view the TUI renders. It is deliberately
// decoupled from the raw endpoint JSON so schema drift stays in this file.
type UsageSnapshot struct {
	Windows   []UsageWindow
	FetchedAt time.Time
}

// rawUsage encodes the shape of GET /api/oauth/usage, pinned against a live
// response (2026-07-22). The self-describing `limits` array is the canonical
// source — it carries all three windows including the model-scoped (Fable) one,
// which the flat top-level fields do NOT (seven_day_opus is null in practice).
// The top-level fields are kept only as a fallback if `limits` ever disappears.
// If the real field names drift, fix them HERE — these tags are the single
// point of coupling to the endpoint.
type rawScope struct {
	Model struct {
		DisplayName string `json:"display_name"`
	} `json:"model"`
}

type rawLimit struct {
	Kind string `json:"kind"` // "session" | "weekly_all" | "weekly_scoped"
	// Percent is a pointer so a limit present but missing its percent (a partial
	// schema drift) is distinguishable from a genuine 0% — the former is skipped
	// rather than rendered as a fabricated 0% bar (the "never fabricate" contract).
	Percent  *float64  `json:"percent"` // 0-100 scale (confirmed live)
	ResetsAt string    `json:"resets_at"`
	Scope    *rawScope `json:"scope"` // non-nil for "weekly_scoped" (holds the model name)
}

type rawUsageWindow struct {
	Utilization *float64 `json:"utilization"` // 0-100 scale (confirmed live)
	ResetsAt    string   `json:"resets_at"`
}

type rawUsage struct {
	Limits []rawLimit `json:"limits"` // primary, self-describing source
	// Legacy flat fields — fallback only, used when `limits` is absent/empty.
	FiveHour *rawUsageWindow `json:"five_hour"`
	SevenDay *rawUsageWindow `json:"seven_day"`
}

// parseUsage converts the raw endpoint body into a normalized snapshot.
// `now` is injected so callers/tests control FetchedAt.
func parseUsage(data []byte, now time.Time) (UsageSnapshot, error) {
	var raw rawUsage
	if err := json.Unmarshal(data, &raw); err != nil {
		return UsageSnapshot{}, fmt.Errorf("cannot parse usage response: %w", err)
	}
	snap := UsageSnapshot{FetchedAt: now}
	add := func(label string, percent *float64, resetsAt string) {
		if label == "" || percent == nil {
			return
		}
		win := UsageWindow{Source: usageClaude, Label: label, Percent: clampPercent(*percent)}
		if t, err := time.Parse(time.RFC3339, resetsAt); err == nil {
			win.ResetsAt = t
		}
		snap.Windows = append(snap.Windows, win)
	}
	// Primary: the self-describing `limits` array, in its natural order.
	for _, l := range raw.Limits {
		add(limitLabel(l), l.Percent, l.ResetsAt)
	}
	// Fallback: the flat top-level fields, only if `limits` yielded nothing.
	if len(snap.Windows) == 0 {
		if raw.FiveHour != nil {
			add("Current session", raw.FiveHour.Utilization, raw.FiveHour.ResetsAt)
		}
		if raw.SevenDay != nil {
			add("Current week (all models)", raw.SevenDay.Utilization, raw.SevenDay.ResetsAt)
		}
	}
	if len(snap.Windows) == 0 {
		return UsageSnapshot{}, errors.New("usage response had no recognized windows")
	}
	return snap, nil
}

// limitLabel maps a limits[] entry to a display label, or "" to skip it. The
// scoped window's label embeds the model display name (e.g. "Fable") so the
// crimson-"Fable" rendering keeps working off the label text.
func limitLabel(l rawLimit) string {
	switch l.Kind {
	case "session":
		return "Current session"
	case "weekly_all":
		return "Current week (all models)"
	case "weekly_scoped":
		name := "scoped"
		if l.Scope != nil && l.Scope.Model.DisplayName != "" {
			name = l.Scope.Model.DisplayName
		}
		return "Current week (" + name + ")"
	default:
		return ""
	}
}

func clampPercent(v float64) int {
	p := int(v + 0.5)
	if p < 0 {
		return 0
	}
	return p
}

// --- providers ---

const (
	usageEndpoint      = "https://api.anthropic.com/api/oauth/usage"
	anthropicBetaOAuth = "oauth-2025-04-20"        // pinned live 2026-07-22
	keychainService    = "Claude Code-credentials" // pinned live 2026-07-22 (-s label for `security`)
	usageFetchTimeout  = 8 * time.Second
)

var (
	errNoCredentials = errors.New("no Claude credentials found")
	errAuthRejected  = errors.New("Claude rejected the credentials")
	errRateLimited   = errors.New("usage endpoint rate-limited the request")
	errNoCodex       = errors.New("Codex CLI not found")
)

// UsageProvider fetches a normalized usage snapshot. The interface lets the TUI
// and tests inject a fake with no network or Keychain access.
type UsageProvider interface {
	Fetch(ctx context.Context) (UsageSnapshot, error)
}

type httpUsageProvider struct {
	client   *http.Client
	tokenFn  func() (string, error)
	endpoint string
}

func (p httpUsageProvider) Fetch(ctx context.Context) (UsageSnapshot, error) {
	token, err := p.tokenFn()
	if err != nil || token == "" {
		return UsageSnapshot{}, errNoCredentials
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, p.endpoint, nil)
	if err != nil {
		return UsageSnapshot{}, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("anthropic-beta", anthropicBetaOAuth)
	req.Header.Set("Accept", "application/json")
	resp, err := p.client.Do(req)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) {
			return UsageSnapshot{}, context.DeadlineExceeded
		}
		return UsageSnapshot{}, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	switch {
	case resp.StatusCode == http.StatusUnauthorized, resp.StatusCode == http.StatusForbidden:
		return UsageSnapshot{}, errAuthRejected
	case resp.StatusCode == http.StatusTooManyRequests:
		return UsageSnapshot{}, errRateLimited
	case resp.StatusCode != http.StatusOK:
		return UsageSnapshot{}, fmt.Errorf("usage endpoint returned status %d", resp.StatusCode)
	}
	return parseUsage(body, time.Now())
}

// fetchRawUsageBody performs the same authenticated GET as the provider but
// returns the raw response body verbatim (no parsing). It exists only for
// `domux usage --raw`, a one-time diagnostic to reveal the real JSON field
// names so the CONFIRM-AT-VERIFY struct tags can be pinned. The body contains
// no token — only usage numbers and reset timestamps — so printing it is safe.
func fetchRawUsageBody(ctx context.Context) ([]byte, error) {
	token, err := readClaudeToken()
	if err != nil || token == "" {
		return nil, errNoCredentials
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, usageEndpoint, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("anthropic-beta", anthropicBetaOAuth)
	req.Header.Set("Accept", "application/json")
	resp, err := (&http.Client{}).Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if resp.StatusCode != http.StatusOK {
		return body, fmt.Errorf("usage endpoint returned status %d", resp.StatusCode)
	}
	return body, nil
}

// fixtureUsageProvider renders a captured JSON file with no network — set
// DOMUX_USAGE_FIXTURE to verify rendering before the live call is confirmed.
type fixtureUsageProvider struct{ path string }

func (p fixtureUsageProvider) Fetch(ctx context.Context) (UsageSnapshot, error) {
	data, err := os.ReadFile(p.path)
	if err != nil {
		return UsageSnapshot{}, err
	}
	return parseUsage(data, time.Now())
}

func newUsageProvider() UsageProvider {
	if path := strings.TrimSpace(os.Getenv("DOMUX_USAGE_FIXTURE")); path != "" {
		return fixtureUsageProvider{path: path}
	}
	return combinedUsageProvider{
		claude: newClaudeUsageProvider(),
		codex:  newCodexUsageProvider(),
	}
}

func newClaudeUsageProvider() UsageProvider {
	// No http.Client.Timeout: the caller (usageFetchCmd) supplies a ctx
	// deadline that propagates via http.NewRequestWithContext, so a timeout
	// unwraps cleanly to context.DeadlineExceeded (a client Timeout would race
	// it and produce an error that does not, muddying the "network timeout"
	// reason the TUI shows).
	live := httpUsageProvider{
		client:   &http.Client{},
		tokenFn:  readClaudeToken,
		endpoint: usageEndpoint,
	}
	// Wrap in a shared on-disk cache so the popup and the picker's poll share
	// one fetch: the endpoint rate-limits aggressively (~1 call/min), and two
	// independent processes hitting it would trip 429s. The cache serves a
	// fresh-enough snapshot without a call and falls back to the last-good one
	// on any error, making rate-limits invisible.
	return cachedUsageProvider{inner: live, ttl: usageCacheTTL, path: usageCachePath}
}

func newCodexUsageProvider() UsageProvider {
	live := codexUsageProvider{command: "codex"}
	return cachedUsageProvider{inner: live, ttl: usageCacheTTL, path: codexUsageCachePath}
}

// combinedUsageProvider fetches Claude and Codex concurrently. One unavailable
// source must not hide the other source's last-good snapshot.
type combinedUsageProvider struct {
	claude UsageProvider
	codex  UsageProvider
}

func (p combinedUsageProvider) Fetch(ctx context.Context) (UsageSnapshot, error) {
	type result struct {
		snap UsageSnapshot
		err  error
	}
	results := make(chan result, 2)
	for _, provider := range []UsageProvider{p.claude, p.codex} {
		go func(provider UsageProvider) {
			if provider == nil {
				results <- result{err: errors.New("usage provider unavailable")}
				return
			}
			snap, err := provider.Fetch(ctx)
			results <- result{snap: snap, err: err}
		}(provider)
	}

	var combined UsageSnapshot
	var errs []error
	for range 2 {
		result := <-results
		if result.err != nil {
			errs = append(errs, result.err)
			continue
		}
		combined.Windows = append(combined.Windows, result.snap.Windows...)
		if result.snap.FetchedAt.After(combined.FetchedAt) {
			combined.FetchedAt = result.snap.FetchedAt
		}
	}
	if len(combined.Windows) == 0 {
		return UsageSnapshot{}, errors.Join(errs...)
	}
	return combined, nil
}

// usageCacheTTL is deliberately conservative: the endpoint's rate limit is
// shared across every Claude Code / domux process on the account, so domux's
// own polling (tmux status bar + picker, potentially from several attached
// clients) needs a wide margin rather than chasing per-minute freshness.
const usageCacheTTL = 5 * time.Minute

// usageCachePath is the shared snapshot-cache location. It holds only a
// normalized UsageSnapshot (percentages + reset times) — never the token.
func usageCachePath() (string, error) {
	return domuxDataDir("usage-cache.json")
}

func codexUsageCachePath() (string, error) {
	return domuxDataDir("codex-usage-cache.json")
}

// codexUsageProvider asks the local Codex CLI app-server for the signed-in
// account's rate-limit snapshot. This keeps OAuth credentials inside Codex;
// domux neither reads nor transmits them.
type codexUsageProvider struct {
	command string
}

type codexRPCMessage struct {
	ID     json.RawMessage `json:"id"`
	Result json.RawMessage `json:"result"`
	Error  *struct {
		Message string `json:"message"`
	} `json:"error"`
}

type codexRateLimitWindow struct {
	UsedPercent int    `json:"usedPercent"`
	ResetsAt    *int64 `json:"resetsAt"`
}

type codexRateLimitResponse struct {
	RateLimits struct {
		Primary   *codexRateLimitWindow `json:"primary"`
		Secondary *codexRateLimitWindow `json:"secondary"`
	} `json:"rateLimits"`
}

func (p codexUsageProvider) Fetch(ctx context.Context) (UsageSnapshot, error) {
	command := p.command
	if command == "" {
		command = "codex"
	}
	if _, err := exec.LookPath(command); err != nil {
		return UsageSnapshot{}, errNoCodex
	}

	cmd := exec.CommandContext(ctx, command, "app-server", "--stdio")
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return UsageSnapshot{}, err
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return UsageSnapshot{}, err
	}
	if err := cmd.Start(); err != nil {
		return UsageSnapshot{}, err
	}
	defer func() {
		_ = stdin.Close()
		_ = cmd.Wait()
	}()

	encoder := json.NewEncoder(stdin)
	decoder := json.NewDecoder(bufio.NewReader(stdout))
	if err := encoder.Encode(map[string]any{
		"id":     1,
		"method": "initialize",
		"params": map[string]any{
			"clientInfo": map[string]string{"name": "domux", "version": version},
		},
	}); err != nil {
		return UsageSnapshot{}, err
	}
	if _, err := readCodexRPCResponse(decoder, 1); err != nil {
		return UsageSnapshot{}, err
	}
	if err := encoder.Encode(map[string]any{
		"id":     2,
		"method": "account/rateLimits/read",
		"params": nil,
	}); err != nil {
		return UsageSnapshot{}, err
	}
	body, err := readCodexRPCResponse(decoder, 2)
	if err != nil {
		return UsageSnapshot{}, err
	}
	return parseCodexRateLimits(body, time.Now())
}

func readCodexRPCResponse(decoder *json.Decoder, wantID int) ([]byte, error) {
	for {
		var message codexRPCMessage
		if err := decoder.Decode(&message); err != nil {
			return nil, err
		}
		var id int
		if len(message.ID) == 0 || json.Unmarshal(message.ID, &id) != nil || id != wantID {
			continue // notification or response to an earlier request
		}
		if message.Error != nil {
			return nil, errors.New(message.Error.Message)
		}
		if len(message.Result) == 0 {
			return nil, errors.New("Codex app-server returned no result")
		}
		return message.Result, nil
	}
}

func parseCodexRateLimits(data []byte, now time.Time) (UsageSnapshot, error) {
	var raw codexRateLimitResponse
	if err := json.Unmarshal(data, &raw); err != nil {
		return UsageSnapshot{}, fmt.Errorf("cannot parse Codex rate limits: %w", err)
	}
	snap := UsageSnapshot{FetchedAt: now}
	add := func(label string, raw *codexRateLimitWindow) {
		if raw == nil {
			return
		}
		win := UsageWindow{Source: usageCodex, Label: label, Percent: clampPercent(float64(raw.UsedPercent))}
		if raw.ResetsAt != nil && *raw.ResetsAt > 0 {
			win.ResetsAt = time.Unix(*raw.ResetsAt, 0)
		}
		snap.Windows = append(snap.Windows, win)
	}
	add("Current session", raw.RateLimits.Primary)
	add("Current week", raw.RateLimits.Secondary)
	if len(snap.Windows) == 0 {
		return UsageSnapshot{}, errors.New("Codex rate limits had no recognized windows")
	}
	return snap, nil
}

// cachedUsageProvider wraps another provider with a TTL'd on-disk snapshot
// cache. A fetch within the TTL returns the cached snapshot with no network
// call; a stale cache triggers one call, and any fetch error falls back to the
// last-good cached snapshot (even if expired) so a transient rate-limit or
// network blip never surfaces as "unavailable" once we've seen real data.
//
// The cache is shared on disk across every domux process on the machine (the
// tmux status bar polls it from every attached client, plus the picker), so a
// stale-cache moment can be observed by several processes at once. Without
// coordination each would fire its own live request at the same instant,
// multiplying domux's share of the endpoint's aggressive rate limit. A
// same-directory lock file lets only one process refresh at a time; the rest
// serve the last-good cached snapshot instead of piling on.
type cachedUsageProvider struct {
	inner UsageProvider
	ttl   time.Duration
	path  func() (string, error)
}

// usageRefreshLockTTL bounds how long a refresh lock is honored. A live fetch
// normally completes well within this; a lock older than this belongs to a
// process that died mid-fetch and is safe to reclaim.
const usageRefreshLockTTL = 20 * time.Second

func (p cachedUsageProvider) Fetch(ctx context.Context) (UsageSnapshot, error) {
	cached, cachedAt, haveCache := p.readCache()
	if haveCache && time.Since(cachedAt) < p.ttl {
		return cached, nil
	}
	if haveCache {
		release, claimed := p.acquireRefreshLock()
		if !claimed {
			// Another local process is already refreshing this same cache;
			// serve the last-good snapshot rather than adding a second
			// concurrent request against the rate limit.
			return cached, nil
		}
		defer release()
	}
	snap, err := p.inner.Fetch(ctx)
	if err != nil {
		if haveCache {
			return cached, nil // serve last-good rather than an error
		}
		return UsageSnapshot{}, err
	}
	p.writeCache(snap)
	return snap, nil
}

// acquireRefreshLock claims the on-disk refresh lock for this cache. It
// reclaims an abandoned lock (older than usageRefreshLockTTL) before trying,
// so a process that died mid-fetch can't wedge refreshes forever. The
// returned release func is a no-op if the lock path can't be resolved, in
// which case claimed is true so callers proceed unlocked rather than block.
func (p cachedUsageProvider) acquireRefreshLock() (release func(), claimed bool) {
	path, err := p.path()
	if err != nil {
		return func() {}, true
	}
	lockPath := path + ".lock"
	if info, err := os.Stat(lockPath); err == nil && time.Since(info.ModTime()) >= usageRefreshLockTTL {
		_ = os.Remove(lockPath)
	}
	f, err := os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0644)
	if err != nil {
		return func() {}, false
	}
	f.Close()
	return func() { _ = os.Remove(lockPath) }, true
}

// usageCacheFile is the on-disk shape: the normalized snapshot plus the wall
// time it was cached (FetchedAt is preserved inside the snapshot too).
type usageCacheFile struct {
	CachedAt time.Time     `json:"cached_at"`
	Snapshot UsageSnapshot `json:"snapshot"`
}

func (p cachedUsageProvider) readCache() (UsageSnapshot, time.Time, bool) {
	path, err := p.path()
	if err != nil {
		return UsageSnapshot{}, time.Time{}, false
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return UsageSnapshot{}, time.Time{}, false
	}
	var cf usageCacheFile
	if err := json.Unmarshal(data, &cf); err != nil || len(cf.Snapshot.Windows) == 0 {
		return UsageSnapshot{}, time.Time{}, false
	}
	return cf.Snapshot, cf.CachedAt, true
}

func (p cachedUsageProvider) writeCache(snap UsageSnapshot) {
	path, err := p.path()
	if err != nil {
		return
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return
	}
	data, err := json.Marshal(usageCacheFile{CachedAt: time.Now(), Snapshot: snap})
	if err != nil {
		return
	}
	// Atomic write (write-tmp-then-rename), per the repo convention.
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0644); err != nil {
		return
	}
	_ = os.Rename(tmp, path)
}

// readClaudeToken resolves the OAuth access token from, in order: an explicit
// env override, the credentials file (Linux + some macOS setups), then the
// macOS Keychain via the `security` CLI (the mechanism Claude Code itself uses).
func readClaudeToken() (string, error) {
	if t := strings.TrimSpace(os.Getenv("DOMUX_CLAUDE_TOKEN")); t != "" {
		return t, nil
	}
	if t, err := tokenFromCredentialsFile(); err == nil && t != "" {
		return t, nil
	}
	if runtime.GOOS == "darwin" {
		if t, err := tokenFromKeychain(); err == nil && t != "" {
			return t, nil
		}
	}
	return "", errNoCredentials
}

func tokenFromCredentialsFile() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	data, err := os.ReadFile(filepath.Join(home, ".claude", ".credentials.json"))
	if err != nil {
		return "", err
	}
	return tokenFromCredentialsJSON(data)
}

func tokenFromKeychain() (string, error) {
	account := os.Getenv("USER") // pinned live 2026-07-22 (-a account label for the Keychain item)
	out, err := exec.Command("security", "find-generic-password", "-s", keychainService, "-a", account, "-w").Output()
	if err != nil {
		return "", err
	}
	return tokenFromCredentialsJSON(out)
}

// tokenFromCredentialsJSON extracts the OAuth access token from the credentials
// blob (the same JSON shape stored in the file and the Keychain item).
func tokenFromCredentialsJSON(data []byte) (string, error) {
	var creds struct {
		ClaudeAIOAuth struct {
			AccessToken string `json:"accessToken"`
		} `json:"claudeAiOauth"`
	}
	if err := json.Unmarshal(data, &creds); err != nil {
		return "", err
	}
	if creds.ClaudeAIOAuth.AccessToken == "" {
		return "", errors.New("no accessToken in credentials")
	}
	return creds.ClaudeAIOAuth.AccessToken, nil
}
