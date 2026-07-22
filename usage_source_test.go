package main

import (
	"context"
	"io"
	"net/http"
	"path/filepath"
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

func TestFetchMapsRateLimit(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		return jsonResponse(429, `{"error":{"type":"rate_limit_error"}}`), nil
	})}
	p := httpUsageProvider{client: client, tokenFn: func() (string, error) { return "tok", nil }, endpoint: usageEndpoint}
	if _, err := p.Fetch(context.Background()); err != errRateLimited {
		t.Fatalf("err = %v, want errRateLimited", err)
	}
}

// countingProvider records how many times Fetch is called and returns a fixed
// result, so cache tests can assert the network was (or wasn't) hit.
type countingProvider struct {
	snap  UsageSnapshot
	err   error
	calls int
}

func (c *countingProvider) Fetch(ctx context.Context) (UsageSnapshot, error) {
	c.calls++
	return c.snap, c.err
}

func snapWith(pct int) UsageSnapshot {
	return UsageSnapshot{Windows: []UsageWindow{{Label: "Current session", Percent: pct}}}
}

func tempCachePath(t *testing.T) func() (string, error) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "usage-cache.json")
	return func() (string, error) { return path, nil }
}

func TestCachedProviderServesFreshCacheWithoutFetch(t *testing.T) {
	inner := &countingProvider{snap: snapWith(10)}
	pathFn := tempCachePath(t)
	p := cachedUsageProvider{inner: inner, ttl: time.Minute, path: pathFn}
	// First call populates the cache (one fetch).
	if _, err := p.Fetch(context.Background()); err != nil {
		t.Fatalf("first fetch: %v", err)
	}
	// Second call within the TTL must be served from cache (no extra fetch).
	snap, err := p.Fetch(context.Background())
	if err != nil {
		t.Fatalf("second fetch: %v", err)
	}
	if inner.calls != 1 {
		t.Fatalf("inner called %d times, want 1 (second read should hit cache)", inner.calls)
	}
	if len(snap.Windows) != 1 || snap.Windows[0].Percent != 10 {
		t.Fatalf("cached snapshot = %#v", snap)
	}
}

func TestCachedProviderFallsBackToCacheOnError(t *testing.T) {
	pathFn := tempCachePath(t)
	// Seed a good snapshot into the cache.
	seed := cachedUsageProvider{inner: &countingProvider{snap: snapWith(42)}, ttl: time.Minute, path: pathFn}
	if _, err := seed.Fetch(context.Background()); err != nil {
		t.Fatalf("seed: %v", err)
	}
	// A provider whose cache is expired (ttl 0) and whose live fetch errors
	// must still serve the last-good cached snapshot, not the error.
	failing := cachedUsageProvider{inner: &countingProvider{err: errRateLimited}, ttl: 0, path: pathFn}
	snap, err := failing.Fetch(context.Background())
	if err != nil {
		t.Fatalf("expected fallback to cache, got error %v", err)
	}
	if len(snap.Windows) != 1 || snap.Windows[0].Percent != 42 {
		t.Fatalf("expected last-good snapshot (42%%), got %#v", snap)
	}
}

func TestCachedProviderPropagatesErrorWhenNoCache(t *testing.T) {
	pathFn := tempCachePath(t)
	p := cachedUsageProvider{inner: &countingProvider{err: errNoCredentials}, ttl: time.Minute, path: pathFn}
	if _, err := p.Fetch(context.Background()); err != errNoCredentials {
		t.Fatalf("err = %v, want errNoCredentials (no cache to fall back to)", err)
	}
}

func TestCachedProviderRefetchesWhenStale(t *testing.T) {
	inner := &countingProvider{snap: snapWith(1)}
	pathFn := tempCachePath(t)
	p := cachedUsageProvider{inner: inner, ttl: 0, path: pathFn} // ttl 0 => always stale
	if _, err := p.Fetch(context.Background()); err != nil {
		t.Fatalf("fetch 1: %v", err)
	}
	if _, err := p.Fetch(context.Background()); err != nil {
		t.Fatalf("fetch 2: %v", err)
	}
	if inner.calls != 2 {
		t.Fatalf("inner called %d times, want 2 (stale cache should refetch)", inner.calls)
	}
}
