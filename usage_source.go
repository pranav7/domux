package main

import (
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

// UsageWindow is one subscription rate-limit window (session / weekly / weekly-Fable).
type UsageWindow struct {
	Label    string
	Percent  int       // 0-100, clamped
	ResetsAt time.Time // zero if the endpoint omitted / sent an unparseable time
}

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
		win := UsageWindow{Label: label, Percent: clampPercent(*percent)}
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
	if p > 100 {
		return 100
	}
	return p
}

// --- provider: token read + HTTP fetch ---

const (
	usageEndpoint      = "https://api.anthropic.com/api/oauth/usage"
	anthropicBetaOAuth = "oauth-2025-04-20"        // pinned live 2026-07-22
	keychainService    = "Claude Code-credentials" // pinned live 2026-07-22 (-s label for `security`)
	usageFetchTimeout  = 8 * time.Second
)

var (
	errNoCredentials = errors.New("no Claude credentials found")
	errAuthRejected  = errors.New("Claude rejected the credentials")
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
	// No http.Client.Timeout: the caller (usageFetchCmd) supplies a ctx
	// deadline that propagates via http.NewRequestWithContext, so a timeout
	// unwraps cleanly to context.DeadlineExceeded (a client Timeout would race
	// it and produce an error that does not, muddying the "network timeout"
	// reason the TUI shows).
	return httpUsageProvider{
		client:   &http.Client{},
		tokenFn:  readClaudeToken,
		endpoint: usageEndpoint,
	}
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
