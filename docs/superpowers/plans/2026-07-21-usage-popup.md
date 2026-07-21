# Usage Popup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `domux usage` tmux popup that shows Claude subscription usage bars (Current session / Current week (all models) / Current week (Fable)) fetched from Claude's authenticated `/api/oauth/usage` endpoint, with an honest "unavailable" state on any failure.

**Architecture:** Three isolated units mirroring the existing `picker.go` + `pr_cache.go` + palette-in-`tui.go` split. `usage_source.go` owns all fragile external contact (Keychain token read + HTTP + JSON parse) and exposes a normalized `UsageSnapshot` behind a `UsageProvider` interface. `usage.go` is a bubbletea model that renders the snapshot as colored bars and knows nothing about HTTP. Wiring adds a `usage` subcommand and a tmux `display-popup` bind-key. The provider is the single fragility-containment point: endpoint/header/schema drift is a one-file fix.

**Tech Stack:** Go 1.22, bubbletea v1.2.4, lipgloss v1.0.0, stdlib `net/http` (the first HTTP client in the repo), `os/exec` (`security` CLI for Keychain — no new dependency).

## Global Constraints

- Go 1.22, single-package layout (`package main`, no module subdirs). Every new file starts `package main`.
- No new third-party dependencies. `net/http`, `encoding/json`, `os/exec`, `context`, `time`, `runtime` are all stdlib and allowed. Keychain access is via the `security` CLI (`exec.Command`), NOT a Keychain library — this matches the repo's "tmux interop is exec.Command only" convention.
- Reuse the Catppuccin Mocha palette vars already declared in `tui.go:49-66` (`green`, `yellow`, `red`, `text`, `subtext0`, `overlay0`, `base`, etc.). Do not redeclare colors that exist.
- The OAuth token is a secret: never log it, never write it to disk, never render it. Read it only on demand (when the popup opens).
- On ANY failure (no token, `security` denied, non-200, timeout, malformed JSON), render an honest "Usage unavailable — <reason>" state. NEVER fabricate or estimate numbers.
- New subcommand registers in `runCommand` (`commands.go:17`). Simple no-arg commands check `len(args) != 0` and return an error (follow the `sessions` case at `commands.go:22`).
- Tests: table-driven, `package main`, match the style in `pr_cache_test.go` (plain `testing`, `t.Fatalf` with `%#v`). No network or Keychain touched in tests — inject fakes.

### Confirm-at-verify constants (isolated, best-guess until pinned)

Three literals are not yet confirmed and are quarantined as named constants in `usage_source.go`. They are read-only discoverable from the Claude binary (`strings <bin> | grep`), but the auto-mode classifier blocks automated credential-store recon, so they are pinned during the verify stage (one-time permission grant or Pranav runs the grep). Until pinned, best-guess values are used; a wrong value surfaces as the honest "unavailable" state, never a crash.

1. `keychainService` — the `-s` service label for `security find-generic-password`. Best guess: `"Claude Code-credentials"`.
2. `anthropicBetaOAuth` — the `anthropic-beta` header value. Best guess: `"oauth-2025-04-20"`.
3. The `/api/oauth/usage` JSON field names — encoded as struct tags in `rawUsage` (Task 1). Best guess: `five_hour` / `seven_day` / `seven_day_opus`, each `{ "utilization": <0-100>, "resets_at": <RFC3339> }`.

A `DOMUX_USAGE_FIXTURE=<path>` env override (Task 3) lets a captured real JSON response drive the popup with no network, so rendering can be verified before the live call is confirmed. A `DOMUX_CLAUDE_TOKEN` env override lets a token be supplied directly, bypassing Keychain.

---

### Task 1: UsageSnapshot types + tolerant parser

**Files:**
- Create: `usage_source.go`
- Test: `usage_source_test.go`

**Interfaces:**
- Consumes: nothing (leaf).
- Produces:
  - `type UsageWindow struct { Label string; Percent int; ResetsAt time.Time }`
  - `type UsageSnapshot struct { Windows []UsageWindow; FetchedAt time.Time }`
  - `func parseUsage(data []byte, now time.Time) (UsageSnapshot, error)`
  - `func clampPercent(v float64) int`

- [ ] **Step 1: Write the failing test**

Create `usage_source_test.go`:

```go
package main

import (
	"testing"
	"time"
)

func TestParseUsageMapsThreeWindows(t *testing.T) {
	body := []byte(`{
		"five_hour":       {"utilization": 15, "resets_at": "2026-07-21T19:29:00Z"},
		"seven_day":       {"utilization": 24, "resets_at": "2026-07-27T19:59:00Z"},
		"seven_day_opus":  {"utilization": 4,  "resets_at": "2026-07-27T19:59:00Z"}
	}`)
	now := time.Date(2026, 7, 21, 12, 0, 0, 0, time.UTC)
	snap, err := parseUsage(body, now)
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if !snap.FetchedAt.Equal(now) {
		t.Fatalf("FetchedAt = %v", snap.FetchedAt)
	}
	if len(snap.Windows) != 3 {
		t.Fatalf("windows = %#v", snap.Windows)
	}
	w := snap.Windows[0]
	if w.Label != "Current session" || w.Percent != 15 {
		t.Fatalf("window[0] = %#v", w)
	}
	if snap.Windows[2].Label != "Current week (Fable)" || snap.Windows[2].Percent != 4 {
		t.Fatalf("window[2] = %#v", snap.Windows[2])
	}
	if snap.Windows[0].ResetsAt.IsZero() {
		t.Fatalf("expected parsed reset time")
	}
}

func TestParseUsageSkipsMissingWindows(t *testing.T) {
	body := []byte(`{"five_hour": {"utilization": 50, "resets_at": "2026-07-21T19:29:00Z"}}`)
	snap, err := parseUsage(body, time.Now())
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if len(snap.Windows) != 1 || snap.Windows[0].Label != "Current session" {
		t.Fatalf("windows = %#v", snap.Windows)
	}
}

func TestParseUsageErrorsOnGarbage(t *testing.T) {
	if _, err := parseUsage([]byte(`not json`), time.Now()); err == nil {
		t.Fatalf("expected error on malformed JSON")
	}
}

func TestParseUsageErrorsWhenNoWindows(t *testing.T) {
	if _, err := parseUsage([]byte(`{}`), time.Now()); err == nil {
		t.Fatalf("expected error when no recognized windows")
	}
}

func TestClampPercent(t *testing.T) {
	cases := map[float64]int{-5: 0, 0: 0, 14.6: 15, 100: 100, 150: 100}
	for in, want := range cases {
		if got := clampPercent(in); got != want {
			t.Fatalf("clampPercent(%v) = %d, want %d", in, got, want)
		}
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestParseUsage ./...`
Expected: FAIL — `undefined: parseUsage` (and `UsageSnapshot`, `clampPercent`).

- [ ] **Step 3: Write minimal implementation**

Create `usage_source.go`:

```go
package main

import (
	"encoding/json"
	"errors"
	"fmt"
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run 'TestParseUsage|TestClampPercent' ./...`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add usage_source.go usage_source_test.go
git commit -m "feat: add usage snapshot types and tolerant parser"
```

---

### Task 2: Bar renderer + color thresholds

**Files:**
- Create: `usage.go`
- Test: `usage_test.go`

**Interfaces:**
- Consumes: palette vars `green`, `yellow`, `red` from `tui.go`.
- Produces:
  - `func renderBar(percent, width int) string` — plain (no ANSI) meter of `━`/`╌`.
  - `func barColor(percent int) lipgloss.Color`
  - `const usageBarWidth = 20`
  - `var fableCrimson = lipgloss.Color("#DC143C")`

- [ ] **Step 1: Write the failing test**

Create `usage_test.go`:

```go
package main

import (
	"strings"
	"testing"
	"unicode/utf8"
)

func TestRenderBarWidthAndFill(t *testing.T) {
	bar := renderBar(50, 20)
	if utf8.RuneCountInString(bar) != 20 {
		t.Fatalf("bar rune count = %d, want 20 (%q)", utf8.RuneCountInString(bar), bar)
	}
	filled := strings.Count(bar, "━")
	empty := strings.Count(bar, "╌")
	if filled != 10 || empty != 10 {
		t.Fatalf("filled=%d empty=%d, want 10/10", filled, empty)
	}
}

func TestRenderBarBoundaries(t *testing.T) {
	if strings.Count(renderBar(0, 20), "━") != 0 {
		t.Fatalf("0%% should have no filled cells")
	}
	if strings.Count(renderBar(100, 20), "━") != 20 {
		t.Fatalf("100%% should fill all cells")
	}
	if strings.Count(renderBar(150, 20), "━") != 20 {
		t.Fatalf("over 100%% must clamp to full")
	}
	if utf8.RuneCountInString(renderBar(50, 0)) != 1 {
		t.Fatalf("width<1 should coerce to a 1-cell bar")
	}
}

func TestBarColorThresholds(t *testing.T) {
	if barColor(69) != green {
		t.Fatalf("69%% should be green")
	}
	if barColor(70) != yellow {
		t.Fatalf("70%% should be yellow")
	}
	if barColor(89) != yellow {
		t.Fatalf("89%% should be yellow")
	}
	if barColor(90) != red {
		t.Fatalf("90%% should be red")
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestRenderBar|TestBarColor' ./...`
Expected: FAIL — `undefined: renderBar`, `undefined: barColor`.

- [ ] **Step 3: Write minimal implementation**

Create `usage.go` (rendering half; the model is added in Task 4):

```go
package main

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

const usageBarWidth = 20

// fableCrimson highlights the word "Fable" wherever it appears. Distinct from
// the palette's pinkish `red` used for >=90% bar pressure.
var fableCrimson = lipgloss.Color("#DC143C")

// renderBar returns a plain (no ANSI) meter: `━` for filled cells, `╌` for
// empty, matching the statusline meter style. Coloring is applied by the caller.
func renderBar(percent, width int) string {
	if width < 1 {
		width = 1
	}
	if percent < 0 {
		percent = 0
	}
	if percent > 100 {
		percent = 100
	}
	filled := percent * width / 100
	if filled > width {
		filled = width
	}
	var b strings.Builder
	for i := 0; i < filled; i++ {
		b.WriteString("━")
	}
	for i := 0; i < width-filled; i++ {
		b.WriteString("╌")
	}
	return b.String()
}

// barColor maps usage pressure to the green->amber->red thresholds used by the
// statusline (green <70, yellow 70-89, red >=90).
func barColor(percent int) lipgloss.Color {
	switch {
	case percent >= 90:
		return red
	case percent >= 70:
		return yellow
	default:
		return green
	}
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run 'TestRenderBar|TestBarColor' ./...`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add usage.go usage_test.go
git commit -m "feat: add usage bar renderer and color thresholds"
```

---

### Task 3: Token read + HTTP provider

**Files:**
- Modify: `usage_source.go` (append)
- Test: `usage_source_test.go` (append)

**Interfaces:**
- Consumes: `parseUsage` (Task 1).
- Produces:
  - `type UsageProvider interface { Fetch(ctx context.Context) (UsageSnapshot, error) }`
  - `func newUsageProvider() UsageProvider`
  - sentinel errors `errNoCredentials`, `errAuthRejected`
  - `func tokenFromCredentialsJSON(data []byte) (string, error)`
  - constants `usageEndpoint`, `anthropicBetaOAuth`, `keychainService`, `usageFetchTimeout`

- [ ] **Step 1: Write the failing test**

Append to `usage_source_test.go`:

```go
import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)
// NOTE: merge these imports into the existing import block; do not duplicate.

// roundTripFunc lets a test stand in for the network without a server.
type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func jsonResponse(status int, body string) *http.Response {
	return &http.Response{
		StatusCode: status,
		Body:       io.NopCloser(strings.NewReader(body)),
		Header:     make(http.Header),
	}
}

func TestTokenFromCredentialsJSON(t *testing.T) {
	tok, err := tokenFromCredentialsJSON([]byte(`{"claudeAiOauth":{"accessToken":"abc123"}}`))
	if err != nil || tok != "abc123" {
		t.Fatalf("token=%q err=%v", tok, err)
	}
	if _, err := tokenFromCredentialsJSON([]byte(`{"claudeAiOauth":{}}`)); err == nil {
		t.Fatalf("expected error when accessToken missing")
	}
}

func TestFetchSetsAuthAndBetaHeaders(t *testing.T) {
	var gotURL, gotAuth, gotBeta string
	client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		gotURL = r.URL.String()
		gotAuth = r.Header.Get("Authorization")
		gotBeta = r.Header.Get("anthropic-beta")
		return jsonResponse(200, `{"five_hour":{"utilization":15,"resets_at":"2026-07-21T19:29:00Z"}}`), nil
	})}
	p := httpUsageProvider{client: client, tokenFn: func() (string, error) { return "tok", nil }, endpoint: usageEndpoint}
	snap, err := p.Fetch(context.Background())
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if gotURL != usageEndpoint {
		t.Fatalf("URL = %q", gotURL)
	}
	if gotAuth != "Bearer tok" {
		t.Fatalf("Authorization = %q", gotAuth)
	}
	if gotBeta != anthropicBetaOAuth {
		t.Fatalf("anthropic-beta = %q", gotBeta)
	}
	if len(snap.Windows) != 1 {
		t.Fatalf("snapshot = %#v", snap)
	}
}

func TestFetchMapsAuthFailure(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		return jsonResponse(401, `{"error":"unauthorized"}`), nil
	})}
	p := httpUsageProvider{client: client, tokenFn: func() (string, error) { return "tok", nil }, endpoint: usageEndpoint}
	if _, err := p.Fetch(context.Background()); err != errAuthRejected {
		t.Fatalf("err = %v, want errAuthRejected", err)
	}
}

func TestFetchMapsMissingToken(t *testing.T) {
	p := httpUsageProvider{
		client:   &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) { return jsonResponse(200, "{}"), nil })},
		tokenFn:  func() (string, error) { return "", errNoCredentials },
		endpoint: usageEndpoint,
	}
	if _, err := p.Fetch(context.Background()); err != errNoCredentials {
		t.Fatalf("err = %v, want errNoCredentials", err)
	}
}

func TestFetchMapsNon200(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		return jsonResponse(500, `oops`), nil
	})}
	p := httpUsageProvider{client: client, tokenFn: func() (string, error) { return "tok", nil }, endpoint: usageEndpoint}
	if _, err := p.Fetch(context.Background()); err == nil || err == errAuthRejected {
		t.Fatalf("expected a generic non-200 error, got %v", err)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestFetch ./...`
Expected: FAIL — `undefined: httpUsageProvider`, `errAuthRejected`, etc.

- [ ] **Step 3: Write minimal implementation**

Append to `usage_source.go` (add `context`, `io`, `net/http`, `os`, `os/exec`, `path/filepath`, `runtime`, `strings` to the import block):

```go
// --- provider: token read + HTTP fetch ---

const (
	usageEndpoint      = "https://api.anthropic.com/api/oauth/usage"
	anthropicBetaOAuth = "oauth-2025-04-20"       // CONFIRM-AT-VERIFY
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
	account := os.Getenv("USER")
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test ./...`
Expected: PASS (all Task 1–3 tests). Also run `go vet ./...` — expect clean.

- [ ] **Step 5: Commit**

```bash
git add usage_source.go usage_source_test.go
git commit -m "feat: add Claude token read and usage HTTP provider"
```

---

### Task 4: bubbletea usage model

**Files:**
- Modify: `usage.go` (append the model)
- Test: `usage_test.go` (append)

**Interfaces:**
- Consumes: `UsageProvider`, `UsageSnapshot` (Task 3/1), `renderBar`/`barColor`/`fableCrimson`/`usageBarWidth` (Task 2), palette vars from `tui.go`, `errNoCredentials`/`errAuthRejected` (Task 3).
- Produces:
  - `type usageModel struct { ... }`
  - `func newUsageModel(p UsageProvider) usageModel`
  - `func (m usageModel) Init() tea.Cmd`, `Update`, `View`
  - `func usageErrorReason(err error) string`

- [ ] **Step 1: Write the failing test**

Append to `usage_test.go` (add imports `context`, `strings`, `github.com/charmbracelet/lipgloss`, `tea "github.com/charmbracelet/bubbletea"`):

```go
type fakeProvider struct {
	snap UsageSnapshot
	err  error
}

func (f fakeProvider) Fetch(ctx context.Context) (UsageSnapshot, error) { return f.snap, f.err }

func loadedTestModel(t *testing.T) usageModel {
	t.Helper()
	snap := UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current session", Percent: 15},
		{Label: "Current week (all models)", Percent: 24},
		{Label: "Current week (Fable)", Percent: 4},
	}}
	m := newUsageModel(fakeProvider{snap: snap})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{snapshot: snap})
	return next.(usageModel)
}

func TestUsageViewRendersBars(t *testing.T) {
	m := loadedTestModel(t)
	out := stripANSI(m.View())
	for _, want := range []string{"Current session", "15% used", "Current week (Fable)", "4% used"} {
		if !strings.Contains(out, want) {
			t.Fatalf("view missing %q:\n%s", want, out)
		}
	}
	if !strings.Contains(out, "━") || !strings.Contains(out, "╌") {
		t.Fatalf("view missing bar glyphs:\n%s", out)
	}
}

func TestUsageViewShowsUnavailableOnError(t *testing.T) {
	m := newUsageModel(fakeProvider{err: errNoCredentials})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{err: errNoCredentials})
	out := stripANSI(next.(usageModel).View())
	if !strings.Contains(out, "Usage unavailable") {
		t.Fatalf("expected unavailable state:\n%s", out)
	}
	if strings.Contains(out, "% used") {
		t.Fatalf("error view must not render fabricated bars:\n%s", out)
	}
}

func TestUsageErrorReason(t *testing.T) {
	if !strings.Contains(usageErrorReason(errNoCredentials), "credentials") {
		t.Fatalf("bad reason for no creds")
	}
	if !strings.Contains(usageErrorReason(errAuthRejected), "re-login") {
		t.Fatalf("bad reason for auth rejected")
	}
	if !strings.Contains(usageErrorReason(context.DeadlineExceeded), "timeout") {
		t.Fatalf("bad reason for timeout")
	}
}

func TestUsageQuitKeys(t *testing.T) {
	m := loadedTestModel(t)
	for _, key := range []string{"esc", "q"} {
		if _, cmd := m.Update(tea.KeyMsg{Type: keyType(key)}); cmd == nil {
			t.Fatalf("key %q should return a quit command", key)
		}
	}
}
```

Add this helper at the bottom of `usage_test.go` (maps the two test keys to `tea.KeyType`):

```go
func keyType(k string) tea.KeyType {
	if k == "esc" {
		return tea.KeyEsc
	}
	return tea.KeyRunes // "q" is delivered as runes; see note in Step 3
}
```

> NOTE: bubbletea delivers `q` as `tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("q")}`. The helper above returns `tea.KeyRunes` for `q` but sets no runes, so adjust the `TestUsageQuitKeys` loop to build the `q` case as `tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("q")}` and the `esc` case as `tea.KeyMsg{Type: tea.KeyEsc}`. (Rewrite the loop body accordingly — keep it explicit rather than clever.)

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestUsageView|TestUsageError|TestUsageQuit' ./...`
Expected: FAIL — `undefined: newUsageModel`, `usageFetchedMsg`, etc.

- [ ] **Step 3: Write minimal implementation**

Append to `usage.go` (add imports `context`, `errors`, `fmt`, `time`, `tea "github.com/charmbracelet/bubbletea"`):

```go
type usageState int

const (
	usageLoading usageState = iota
	usageLoaded
	usageErr
)

type usageModel struct {
	provider UsageProvider
	state    usageState
	snapshot UsageSnapshot
	err      error
	width    int
	height   int
}

type usageFetchedMsg struct {
	snapshot UsageSnapshot
	err      error
}

func newUsageModel(p UsageProvider) usageModel {
	return usageModel{provider: p, state: usageLoading}
}

func (m usageModel) Init() tea.Cmd {
	return usageFetchCmd(m.provider)
}

func usageFetchCmd(p UsageProvider) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), usageFetchTimeout)
		defer cancel()
		snap, err := p.Fetch(ctx)
		return usageFetchedMsg{snapshot: snap, err: err}
	}
}

func (m usageModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		return m, nil
	case usageFetchedMsg:
		if msg.err != nil {
			m.state, m.err = usageErr, msg.err
		} else {
			m.state, m.snapshot = usageLoaded, msg.snapshot
		}
		return m, nil
	case tea.KeyMsg:
		switch msg.String() {
		case "esc", "q", "ctrl+c":
			return m, tea.Quit
		case "r":
			m.state = usageLoading
			return m, usageFetchCmd(m.provider)
		}
	}
	return m, nil
}

func usageErrorReason(err error) string {
	switch {
	case errors.Is(err, errNoCredentials):
		return "no credentials found — log in with Claude"
	case errors.Is(err, errAuthRejected):
		return "auth rejected — re-login in Claude"
	case errors.Is(err, context.DeadlineExceeded):
		return "network timeout"
	default:
		return "unexpected response"
	}
}

var (
	uTitle    = lipgloss.NewStyle().Foreground(text).Bold(true)
	uLabel    = lipgloss.NewStyle().Foreground(subtext0)
	uFable    = lipgloss.NewStyle().Foreground(fableCrimson).Bold(true)
	uReset    = lipgloss.NewStyle().Foreground(overlay0)
	uFooter   = lipgloss.NewStyle().Foreground(overlay0)
	uErrStyle = lipgloss.NewStyle().Foreground(red)
)

func (m usageModel) View() string {
	if m.width == 0 {
		return ""
	}
	var b strings.Builder
	b.WriteString(uTitle.Render("Claude usage"))
	b.WriteString("\n\n")
	switch m.state {
	case usageLoading:
		b.WriteString(uLabel.Render("Fetching usage…"))
	case usageErr:
		b.WriteString(uErrStyle.Render("Usage unavailable") + uLabel.Render(" — "+usageErrorReason(m.err)))
	case usageLoaded:
		for _, w := range m.snapshot.Windows {
			b.WriteString(renderUsageLabel(w.Label) + "\n")
			bar := lipgloss.NewStyle().Foreground(barColor(w.Percent)).Render(renderBar(w.Percent, usageBarWidth))
			line := fmt.Sprintf("%s  %d%% used", bar, w.Percent)
			if !w.ResetsAt.IsZero() {
				line += uReset.Render("   Resets " + w.ResetsAt.Local().Format("Jan 2 3:04pm"))
			}
			b.WriteString(line + "\n\n")
		}
	}
	b.WriteString("\n" + uFooter.Render("r refresh · esc close"))
	return b.String()
}

// renderUsageLabel colors the word "Fable" crimson wherever it appears; the
// rest of the label uses the muted label style.
func renderUsageLabel(label string) string {
	const fable = "Fable"
	i := strings.Index(label, fable)
	if i < 0 {
		return uLabel.Render(label)
	}
	return uLabel.Render(label[:i]) + uFable.Render(fable) + uLabel.Render(label[i+len(fable):])
}
```

> NOTE on the reset-time format: `time` is imported for the `time.Time` field usage; the format string `"Jan 2 3:04pm"` yields e.g. `Jul 27 8:59pm`. Adjust only if the self-review wants am/pm casing tweaks.

- [ ] **Step 4: Run test to verify it passes**

Run: `go test ./...`
Expected: PASS (all tests). Run `go vet ./...` — expect clean.

- [ ] **Step 5: Commit**

```bash
git add usage.go usage_test.go
git commit -m "feat: add usage bubbletea model with honest failure state"
```

---

### Task 5: Wire the `usage` subcommand + tmux bind-key

**Files:**
- Modify: `usage.go` (append `runUsage`)
- Modify: `commands.go:17` (add `case "usage"`)
- Modify: `main.go` `printUsage` (add help line)
- Modify: `install.go` `generatedTmuxConfig` (add bind-key)

**Interfaces:**
- Consumes: `newUsageModel`, `newUsageProvider`.
- Produces: `func runUsage() error`.

- [ ] **Step 1: Add `runUsage` to `usage.go`**

Append (add `tea "github.com/charmbracelet/bubbletea"` — already imported in Task 4):

```go
func runUsage() error {
	m := newUsageModel(newUsageProvider())
	p := tea.NewProgram(m, tea.WithAltScreen())
	_, err := p.Run()
	return err
}
```

- [ ] **Step 2: Register the subcommand in `commands.go`**

In the `runCommand` switch (`commands.go:17`), add a case next to `sessions` (follow the no-arg guard style):

```go
	case "usage":
		if len(args) != 0 {
			return fmt.Errorf("usage does not accept arguments")
		}
		return runUsage()
```

- [ ] **Step 3: Add the help line in `main.go`**

In `printUsage`, after the `server` line (`main.go:27` region), add:

```go
	fmt.Fprintf(os.Stderr, "  domux usage        Show Claude subscription usage (session/week/Fable)\n")
```

- [ ] **Step 4: Add the tmux bind-key in `install.go`**

In `generatedTmuxConfig`, after the `bind-key u ... commands` line (`install.go:86`), add:

```
bind-key U display-popup -E -w 60% -h 60% "$HOME/bin/domux usage"
```

(`U` is free; `u` is already `commands`, `R` was taken by `clear-window-name`.)

- [ ] **Step 5: Build, vet, test, manual smoke**

```bash
go build ./... && go vet ./... && go test ./...
```
Expected: all pass.

Manual smoke (no live creds needed — uses a fixture):

```bash
# Craft a fixture that matches the best-guess schema
cat > /tmp/usage-fixture.json <<'JSON'
{"five_hour":{"utilization":15,"resets_at":"2026-07-21T19:29:00Z"},
 "seven_day":{"utilization":24,"resets_at":"2026-07-27T19:59:00Z"},
 "seven_day_opus":{"utilization":4,"resets_at":"2026-07-27T19:59:00Z"}}
JSON
DOMUX_USAGE_FIXTURE=/tmp/usage-fixture.json go run . usage
```
Expected: the popup renders three colored bars (session green, week green, Fable green with a crimson "Fable" label) and reset times; `r` refetches, `esc`/`q` closes.

- [ ] **Step 6: Commit**

```bash
git add usage.go commands.go main.go install.go
git commit -m "feat: wire domux usage subcommand and tmux bind-key"
```

---

---

### Task 6: Switcher top-right usage indicator (mid-run scope addition)

**Added mid-run** (user request + screenshot): render a compact 3-segment usage indicator
in the picker's top-right corner — the three limits as short percentages, each in its own
pressure color. Reuses the Task 3 `UsageProvider` and Task 2 `barColor`.

**Files:**
- Modify: `usage.go` (add pure `renderUsageIndicator`)
- Modify: `picker.go` (model fields, slow-poll cmds, wire into Init/Update, right-align in `logoHeaderLines`)
- Test: `usage_test.go` (indicator string) + `picker_test.go` (logo integration)

**Interfaces:**
- Consumes: `UsageProvider`, `UsageSnapshot`, `barColor`, `uLabel`/`uFable`, `newUsageProvider`.
- Produces: `func renderUsageIndicator(snap UsageSnapshot) string`, `func usageTag(label string) string`.

**Design:**
- `renderUsageIndicator` is pure: `""` for an empty snapshot (silent degrade — NEVER fabricate),
  else `<tag> <pct>%` per window joined by ` · `. Tag from label: contains "Fable"→"fab"
  (crimson via `uFable`), contains "session"→"ses", else "wk". `<pct>%` colored by `barColor`.
- Picker gains `usageProvider UsageProvider` and `usage *UsageSnapshot` (nil until first
  success). Slow poll mirrors the PR-refresh cycle: `pickerUsageRefreshCmd` (a `func() tea.Msg`
  that Fetches off the render thread) on Init → on `pickerUsageRefreshMsg`, store snapshot (only
  on success; keep last-good on error) and schedule `pickerUsageTickCmd` → on
  `pickerUsageTickMsg`, re-fetch. `const pickerUsageRefreshInterval = 60 * time.Second`.
  Fetch uses a `context.WithTimeout(usageFetchTimeout)` — never blocks the 2s session tick.
- `logoHeaderLines` right-aligns the indicator on the feature line (logo line 1, the
  "switcher" line) so it never collides with the transient status toast on line 0. Hidden
  when `m.usage == nil`.
- `newPickerModel` defaults `usageProvider = newUsageProvider()`.

Steps: TDD `renderUsageIndicator`/`usageTag` (empty→"", 3 windows→"ses 15% · wk 24% · fab 4%"
under `stripANSI`, tag mapping); then wire the picker fields/cmds/logo; add a `picker_test.go`
case asserting `logoHeaderLines` contains the indicator text when `usage` is set and omits it
when nil; build/vet/test; commit `feat: show usage indicator in switcher top-right`.

---

## Self-Review

**Spec coverage:**
- Provider isolation / fragility containment → Tasks 1 & 3 (`usage_source.go`, one-file schema coupling). ✓
- Endpoint + Bearer + anthropic-beta → Task 3 (`Fetch`, tested headers). ✓
- Keychain via `security` CLI, no new dep → Task 3 (`tokenFromKeychain`). ✓
- Normalized `UsageSnapshot` contract → Task 1. ✓
- Bars in statusline `━`/`╌` green→amber→red style → Task 2. ✓
- Fable label crimson → Task 2 (`fableCrimson`) + Task 4 (`renderUsageLabel`). ✓
- Local-tz reset times → Task 4 (`.Local().Format(...)`). ✓
- `r` refresh / `esc`,`q`,`ctrl+c` close → Task 4. ✓
- Loading + honest "Usage unavailable — reason", never fabricated → Task 4 (tested). ✓
- Token never logged/persisted/rendered → held only in provider locals; no log calls. ✓
- Command + help + bind-key → Task 5. ✓
- v1 = bars only; breakdown deferred → no task builds the breakdown panel. ✓
- Tests: table-driven, fakes, no network/Keychain → all test steps inject `roundTripFunc`/`fakeProvider`. ✓

**Placeholder scan:** No TBD/TODO in code steps; the three confirm-at-verify constants have concrete best-guess values + `CONFIRM-AT-VERIFY` markers and a fixture escape hatch — they are working defaults, not placeholders. ✓

**Type consistency:** `UsageProvider.Fetch(ctx) (UsageSnapshot, error)`, `UsageSnapshot.Windows []UsageWindow`, `usageFetchedMsg{snapshot, err}`, `renderBar(percent,width)`, `barColor(percent)`, `newUsageModel(UsageProvider)`, `newUsageProvider() UsageProvider` — all consistent across Tasks 1–5. ✓
