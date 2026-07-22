package main

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

// liveLimitsBody mirrors the real GET /api/oauth/usage shape (pinned 2026-07-22):
// three windows in the self-describing `limits` array, with the Fable window
// carried as a model-scoped entry (kind "weekly_scoped").
const liveLimitsBody = `{
	"five_hour":      {"utilization": 33, "resets_at": "2026-07-22T14:19:00Z"},
	"seven_day":      {"utilization": 50, "resets_at": "2026-07-27T19:59:00Z"},
	"seven_day_opus": null,
	"limits": [
		{"kind": "session",       "percent": 33, "resets_at": "2026-07-22T14:19:00Z", "scope": null},
		{"kind": "weekly_all",    "percent": 50, "resets_at": "2026-07-27T19:59:00Z", "scope": null},
		{"kind": "weekly_scoped", "percent": 24, "resets_at": "2026-07-27T19:59:00Z",
			"scope": {"model": {"display_name": "Fable"}}}
	]
}`

func TestParseUsageMapsThreeWindows(t *testing.T) {
	now := time.Date(2026, 7, 22, 12, 0, 0, 0, time.UTC)
	snap, err := parseUsage([]byte(liveLimitsBody), now)
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
	if w.Label != "Current session" || w.Percent != 33 {
		t.Fatalf("window[0] = %#v", w)
	}
	// The Fable window comes from the model-scoped limit, NOT seven_day_opus
	// (which is null live) — its label must embed "Fable" for crimson rendering.
	if snap.Windows[2].Label != "Current week (Fable)" || snap.Windows[2].Percent != 24 {
		t.Fatalf("window[2] = %#v", snap.Windows[2])
	}
	if snap.Windows[0].ResetsAt.IsZero() {
		t.Fatalf("expected parsed reset time")
	}
}

func TestParseUsageFallsBackToFlatFields(t *testing.T) {
	// If the `limits` array is ever absent, the flat top-level fields still map.
	body := []byte(`{
		"five_hour": {"utilization": 15, "resets_at": "2026-07-21T19:29:00Z"},
		"seven_day": {"utilization": 24, "resets_at": "2026-07-27T19:59:00Z"}
	}`)
	snap, err := parseUsage(body, time.Now())
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if len(snap.Windows) != 2 || snap.Windows[0].Label != "Current session" {
		t.Fatalf("windows = %#v", snap.Windows)
	}
}

func TestParseUsageSkipsLimitMissingPercent(t *testing.T) {
	// A limit present but missing its percent is a partial schema drift — it
	// must be skipped, not rendered as a fabricated 0%.
	body := []byte(`{"limits": [
		{"kind": "session",    "percent": 15, "resets_at": "2026-07-21T19:29:00Z"},
		{"kind": "weekly_all", "resets_at": "2026-07-27T19:59:00Z"}
	]}`)
	snap, err := parseUsage(body, time.Now())
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if len(snap.Windows) != 1 || snap.Windows[0].Label != "Current session" {
		t.Fatalf("expected only the session window, got %#v", snap.Windows)
	}
}

func TestParseUsageSkipsUnknownLimitKinds(t *testing.T) {
	// Kinds we don't recognize (e.g. new scoped models) are skipped, not shown
	// under a wrong label.
	body := []byte(`{"limits": [
		{"kind": "session",      "percent": 10, "resets_at": "2026-07-21T19:29:00Z"},
		{"kind": "future_thing",  "percent": 99, "resets_at": "2026-07-27T19:59:00Z"}
	]}`)
	snap, err := parseUsage(body, time.Now())
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if len(snap.Windows) != 1 || snap.Windows[0].Label != "Current session" {
		t.Fatalf("expected only the session window, got %#v", snap.Windows)
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
