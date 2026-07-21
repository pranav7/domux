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

// rawUsageWindow / rawUsage encode the (best-guess, confirm-at-verify) shape of
// GET /api/oauth/usage. If the real field names differ, fix them HERE — the
// struct tags are the single source of coupling to the endpoint.
type rawUsageWindow struct {
	Utilization float64 `json:"utilization"` // NOTE: assumed 0-100. If the API sends 0..1 fractions, multiply by 100 in parseUsage.
	ResetsAt    string  `json:"resets_at"`
}

type rawUsage struct {
	FiveHour     *rawUsageWindow `json:"five_hour"`
	SevenDay     *rawUsageWindow `json:"seven_day"`
	SevenDayOpus *rawUsageWindow `json:"seven_day_opus"`
}

// parseUsage converts the raw endpoint body into a normalized snapshot.
// `now` is injected so callers/tests control FetchedAt.
func parseUsage(data []byte, now time.Time) (UsageSnapshot, error) {
	var raw rawUsage
	if err := json.Unmarshal(data, &raw); err != nil {
		return UsageSnapshot{}, fmt.Errorf("cannot parse usage response: %w", err)
	}
	snap := UsageSnapshot{FetchedAt: now}
	add := func(label string, w *rawUsageWindow) {
		if w == nil {
			return
		}
		win := UsageWindow{Label: label, Percent: clampPercent(w.Utilization)}
		if t, err := time.Parse(time.RFC3339, w.ResetsAt); err == nil {
			win.ResetsAt = t
		}
		snap.Windows = append(snap.Windows, win)
	}
	add("Current session", raw.FiveHour)
	add("Current week (all models)", raw.SevenDay)
	add("Current week (Fable)", raw.SevenDayOpus)
	if len(snap.Windows) == 0 {
		return UsageSnapshot{}, errors.New("usage response had no recognized windows")
	}
	return snap, nil
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
	anthropicBetaOAuth = "oauth-2025-04-20"        // CONFIRM-AT-VERIFY
	keychainService    = "Claude Code-credentials" // CONFIRM-AT-VERIFY (-s label for `security`)
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
	return httpUsageProvider{
		client:   &http.Client{Timeout: usageFetchTimeout},
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
	account := os.Getenv("USER") // CONFIRM-AT-VERIFY (-a account label for the Keychain item)
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
