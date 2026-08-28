package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

func TestArtboardsResolvePathNormalFile(t *testing.T) {
	root := t.TempDir()
	groupDir := filepath.Join(root, "group")
	mustMkdirAll(t, groupDir)
	mustWrite(t, filepath.Join(groupDir, "Main.dc.html"), "<html></html>")

	resolved, err := artboardsResolvePath(root, "/group/Main.dc.html")
	if err != nil {
		t.Fatalf("artboardsResolvePath: %v", err)
	}
	if filepath.Base(resolved) != "Main.dc.html" {
		t.Fatalf("unexpected resolved path: %s", resolved)
	}
}

func TestArtboardsResolvePathFollowsSymlinkedGroup(t *testing.T) {
	root := t.TempDir()
	realDir := t.TempDir()
	mustWrite(t, filepath.Join(realDir, "Main.dc.html"), "<html></html>")
	if err := os.Symlink(realDir, filepath.Join(root, "group")); err != nil {
		t.Fatalf("Symlink: %v", err)
	}

	resolved, err := artboardsResolvePath(root, "/group/Main.dc.html")
	if err != nil {
		t.Fatalf("artboardsResolvePath: %v", err)
	}
	// t.TempDir() can itself sit behind a symlink (e.g. macOS /var -> /private/var),
	// so compare against realDir's own resolved form rather than its raw path.
	wantDir, err := filepath.EvalSymlinks(realDir)
	if err != nil {
		t.Fatalf("EvalSymlinks(realDir): %v", err)
	}
	if filepath.Dir(resolved) != wantDir {
		t.Fatalf("expected the symlinked group's real target %s, got %s", wantDir, resolved)
	}
}

func TestArtboardsResolvePathBlocksDotDotEscape(t *testing.T) {
	root := t.TempDir()
	groupDir := filepath.Join(root, "group")
	mustMkdirAll(t, groupDir)
	mustWrite(t, filepath.Join(groupDir, "Main.dc.html"), "<html></html>")

	secretParent := filepath.Dir(root)
	mustWrite(t, filepath.Join(secretParent, "secret.html"), "top secret")
	t.Cleanup(func() { _ = os.Remove(filepath.Join(secretParent, "secret.html")) })

	if _, err := artboardsResolvePath(root, "/group/../../secret.html"); err == nil {
		t.Fatalf("expected a .. escape to be blocked")
	}
}

func TestArtboardsResolvePathBlocksSymlinkEscapeInsideGroup(t *testing.T) {
	root := t.TempDir()
	groupDir := filepath.Join(root, "group")
	mustMkdirAll(t, groupDir)

	outsideDir := t.TempDir()
	mustWrite(t, filepath.Join(outsideDir, "secret.html"), "top secret")
	if err := os.Symlink(outsideDir, filepath.Join(groupDir, "escape")); err != nil {
		t.Fatalf("Symlink: %v", err)
	}

	if _, err := artboardsResolvePath(root, "/group/escape/secret.html"); err == nil {
		t.Fatalf("expected a symlink pointing outside the group to be blocked")
	}
}

func TestArtboardsResolvePathBlocksDisallowedExtension(t *testing.T) {
	root := t.TempDir()
	groupDir := filepath.Join(root, "group")
	mustMkdirAll(t, groupDir)
	mustWrite(t, filepath.Join(groupDir, ".env"), "SECRET=1")
	mustWrite(t, filepath.Join(groupDir, "script.js"), "alert(1)")

	if _, err := artboardsResolvePath(root, "/group/.env"); err == nil {
		t.Fatalf("expected .env to be blocked")
	}
	if _, err := artboardsResolvePath(root, "/group/script.js"); err == nil {
		t.Fatalf("expected .js to be blocked - the artboards format never needs a local script")
	}
}

func TestArtboardsResolvePathBlocksDotfileGroup(t *testing.T) {
	root := t.TempDir()
	dotDir := filepath.Join(root, ".server-state")
	mustMkdirAll(t, dotDir)
	mustWrite(t, filepath.Join(dotDir, "leak.html"), "<html></html>")

	if _, err := artboardsResolvePath(root, "/.server-state/leak.html"); err == nil {
		t.Fatalf("expected a dotfile-prefixed group segment to be blocked")
	}
}

func TestArtboardsResolvePathBlocksBareDirectory(t *testing.T) {
	root := t.TempDir()
	groupDir := filepath.Join(root, "group")
	mustMkdirAll(t, filepath.Join(groupDir, "subdir"))

	if _, err := artboardsResolvePath(root, "/group/subdir"); err == nil {
		t.Fatalf("expected a bare directory request to be blocked (no listing)")
	}
	if _, err := artboardsResolvePath(root, "/group"); err == nil {
		t.Fatalf("expected a bare group request to be blocked (no listing)")
	}
}

func TestArtboardsMuxHealthzDoesNotRegenIndex(t *testing.T) {
	root := t.TempDir()
	srv := httptest.NewServer(artboardsMux(root))
	defer srv.Close()

	// Prime index.html via a real "/" hit, then capture its mtime.
	get(t, srv.URL+"/")
	info, err := os.Stat(filepath.Join(root, "index.html"))
	if err != nil {
		t.Fatalf("stat index.html: %v", err)
	}
	before := info.ModTime()

	resp := get(t, srv.URL+"/healthz")
	if resp.status != http.StatusOK {
		t.Fatalf("expected /healthz to return 200, got %d", resp.status)
	}

	info, err = os.Stat(filepath.Join(root, "index.html"))
	if err != nil {
		t.Fatalf("stat index.html: %v", err)
	}
	if !info.ModTime().Equal(before) {
		t.Fatalf("expected /healthz to never regenerate index.html")
	}
}

func TestArtboardsMuxServesGroupFileAndBlocksTraversal(t *testing.T) {
	root := t.TempDir()
	groupDir := filepath.Join(root, "group")
	mustMkdirAll(t, groupDir)
	mustWrite(t, filepath.Join(groupDir, "Main.dc.html"), "<html>hi</html>")

	srv := httptest.NewServer(artboardsMux(root))
	defer srv.Close()

	resp := get(t, srv.URL+"/group/Main.dc.html")
	if resp.status != http.StatusOK || resp.body != "<html>hi</html>" {
		t.Fatalf("expected the artboard file to serve, got status=%d body=%q", resp.status, resp.body)
	}

	resp = get(t, srv.URL+"/group/%2e%2e/%2e%2e/etc/passwd")
	if resp.status == http.StatusOK {
		t.Fatalf("expected an encoded traversal attempt to be blocked, got 200")
	}
}

type httpResult struct {
	status int
	body   string
}

func get(t *testing.T, url string) httpResult {
	t.Helper()
	resp, err := http.Get(url)
	if err != nil {
		t.Fatalf("GET %s: %v", url, err)
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatalf("read body: %v", err)
	}
	return httpResult{status: resp.StatusCode, body: string(data)}
}

func mustMkdirAll(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(path, 0755); err != nil {
		t.Fatalf("MkdirAll %s: %v", path, err)
	}
}
