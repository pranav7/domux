package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// withArtboardsHealthURL swaps artboardsHealthURL to point at an
// httptest.Server for the duration of the test, mirroring
// withTempCaffeinatePID's package-var swap convention.
func withArtboardsHealthURL(t *testing.T, url string) {
	t.Helper()
	orig := artboardsHealthURL
	artboardsHealthURL = func() string { return url }
	t.Cleanup(func() { artboardsHealthURL = orig })
}

func TestProbeArtboardsHealthFailureThreshold(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)

	up := false
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if up {
			w.WriteHeader(http.StatusOK)
		} else {
			w.WriteHeader(http.StatusServiceUnavailable)
		}
	}))
	defer srv.Close()
	withArtboardsHealthURL(t, srv.URL)

	down, state := probeArtboardsHealth(context.Background())
	if down {
		t.Fatalf("expected 1 failure to stay up, got down=%v state=%+v", down, state)
	}
	if state.ConsecutiveFailures != 1 {
		t.Fatalf("expected 1 consecutive failure, got %d", state.ConsecutiveFailures)
	}

	down, state = probeArtboardsHealth(context.Background())
	if !down {
		t.Fatalf("expected 2 consecutive failures to flip down, got up state=%+v", state)
	}

	up = true
	down, state = probeArtboardsHealth(context.Background())
	if down {
		t.Fatalf("expected a success to reset back to up, got down state=%+v", state)
	}
	if state.ConsecutiveFailures != 0 {
		t.Fatalf("expected the failure counter to reset to 0, got %d", state.ConsecutiveFailures)
	}
}

func TestArtboardsHealthStatePersistsAcrossReload(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer srv.Close()
	withArtboardsHealthURL(t, srv.URL)

	probeArtboardsHealth(context.Background())
	reloaded := loadArtboardsHealthState()
	if reloaded.ConsecutiveFailures != 1 {
		t.Fatalf("expected the failure count to survive a reload, got %d", reloaded.ConsecutiveFailures)
	}
}

func TestProbeArtboardsHealthUnreachableServer(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	withArtboardsHealthURL(t, "http://127.0.0.1:1/healthz") // nothing listens here

	down, _ := probeArtboardsHealth(context.Background())
	if down {
		t.Fatalf("expected the first failure to stay up (debounced)")
	}
	down, state := probeArtboardsHealth(context.Background())
	if !down {
		t.Fatalf("expected a second consecutive connection failure to flip down")
	}
	if state.LastError == "" {
		t.Fatalf("expected LastError to be recorded on a connection failure")
	}
}

func TestArtboardsPlistContentShape(t *testing.T) {
	content := artboardsPlistContent("/usr/local/bin/domux", "/tmp/artboards.log")
	for _, want := range []string{
		"<key>Label</key>",
		"<string>dev.domux.artboards</string>",
		"<string>/usr/local/bin/domux</string>",
		"<string>artboards</string>",
		"<string>serve</string>",
		"<key>RunAtLoad</key>",
		"<true/>",
		"<key>KeepAlive</key>",
		"<string>/tmp/artboards.log</string>",
	} {
		if !strings.Contains(content, want) {
			t.Fatalf("expected plist content to contain %q, got:\n%s", want, content)
		}
	}
}
