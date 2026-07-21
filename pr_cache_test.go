package main

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestParseGHPRListMapsDraft(t *testing.T) {
	pr, ok, err := parseGHPRList([]byte(`[{"number":2,"state":"OPEN","title":"fix\ntitle","isDraft":true}]`))
	if err != nil {
		t.Fatalf("parseGHPRList: %v", err)
	}
	if !ok {
		t.Fatalf("expected PR")
	}
	if pr.Number != 2 || pr.State != "DRAFT" || pr.Title != "fix title" {
		t.Fatalf("PR = %#v", pr)
	}
}

func TestParseGHPRListEmpty(t *testing.T) {
	pr, ok, err := parseGHPRList([]byte(`[]`))
	if err != nil {
		t.Fatalf("parseGHPRList: %v", err)
	}
	if ok || pr != nil {
		t.Fatalf("PR = %#v, ok = %v", pr, ok)
	}
}

func TestRefreshPRCachesWritesDedupeAndPrunes(t *testing.T) {
	homeDir := t.TempDir()
	mustWrite(t, filepath.Join(homeDir, ".tmux-pr-dead"), "9::OPEN::dead\n")
	mustWrite(t, filepath.Join(homeDir, ".tmux-pr-nopr"), "8::OPEN::old\n")
	mustWrite(t, filepath.Join(homeDir, ".tmux-pr-detached"), "7::OPEN::old\n")

	sessions := []prRefreshSession{
		{Name: "s1", Path: "/repo", Root: "/repo", Branch: "feature"},
		{Name: "s2", Path: "/repo/sub", Root: "/repo", Branch: "feature"},
		{Name: "nopr", Path: "/repo", Root: "/repo", Branch: "main"},
		{Name: "detached", Path: "/repo", Root: "/repo"},
	}
	counts := map[string]int{}
	lookup := func(path, branch string) (*prInfo, bool, error) {
		counts[branch]++
		switch branch {
		case "feature":
			return &prInfo{Number: 7, State: "OPEN", Title: "Fix :: thing\nnow"}, true, nil
		case "main":
			return nil, false, nil
		default:
			t.Fatalf("unexpected lookup %s %s", path, branch)
			return nil, false, nil
		}
	}

	if err := refreshPRCachesForSessions(homeDir, sessions, lookup); err != nil {
		t.Fatalf("refreshPRCachesForSessions: %v", err)
	}

	want := "7::OPEN::Fix :: thing now\n"
	if got := mustRead(t, filepath.Join(homeDir, ".tmux-pr-s1")); got != want {
		t.Fatalf("s1 cache = %q", got)
	}
	if got := mustRead(t, filepath.Join(homeDir, ".tmux-pr-s2")); got != want {
		t.Fatalf("s2 cache = %q", got)
	}
	if counts["feature"] != 1 {
		t.Fatalf("feature lookups = %d", counts["feature"])
	}
	if counts["main"] != 1 {
		t.Fatalf("main lookups = %d", counts["main"])
	}
	assertMissing(t, filepath.Join(homeDir, ".tmux-pr-dead"))
	assertMissing(t, filepath.Join(homeDir, ".tmux-pr-nopr"))
	assertMissing(t, filepath.Join(homeDir, ".tmux-pr-detached"))
}

func TestCurrentPRRefreshSessionsSkipsDefaultBranch(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)

	root := setupGitWorkspaceRepo(t)

	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1 $2" in
"list-sessions -F") echo "onmain" ;;
"display-message -t") echo "`+root+`" ;;
esac
exit 0
`, callFile)

	sessions, err := currentPRRefreshSessions()
	if err != nil {
		t.Fatalf("currentPRRefreshSessions: %v", err)
	}
	if len(sessions) != 1 {
		t.Fatalf("sessions = %#v, want 1", sessions)
	}
	// A session sitting on the repo's default branch has no PR of its own;
	// `gh pr list --head main` would match any stale historical PR opened
	// head=main instead, so the branch must be blanked to skip the lookup.
	if sessions[0].Branch != "" {
		t.Fatalf("Branch = %q, want empty for default-branch session", sessions[0].Branch)
	}
}

func TestRefreshPRCachesKeepsCacheOnLookupError(t *testing.T) {
	homeDir := t.TempDir()
	path := filepath.Join(homeDir, ".tmux-pr-s1")
	mustWrite(t, path, "3::OPEN::old\n")

	err := refreshPRCachesForSessions(homeDir, []prRefreshSession{
		{Name: "s1", Path: "/repo", Root: "/repo", Branch: "feature"},
	}, func(path, branch string) (*prInfo, bool, error) {
		return nil, false, errors.New("boom")
	})
	if err == nil {
		t.Fatalf("expected error")
	}
	if got := mustRead(t, path); got != "3::OPEN::old\n" {
		t.Fatalf("cache changed: %q", got)
	}
}

func mustWrite(t *testing.T, path, contents string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(contents), 0644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func mustRead(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	return string(data)
}

func assertMissing(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("expected %s missing, stat err = %v", path, err)
	}
}
