package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestClaudeProjectDirName(t *testing.T) {
	cases := map[string]string{
		"/Users/p/projects/domux":          "-Users-p-projects-domux",
		"/Users/p/projects/a/app/.wt/ws-1": "-Users-p-projects-a-app--wt-ws-1",
		"/a_b/c.d":                         "-a-b-c-d",
		"plain":                            "plain",
	}
	for in, want := range cases {
		if got := claudeProjectDirName(in); got != want {
			t.Errorf("claudeProjectDirName(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestPidAlive(t *testing.T) {
	if !pidAlive(os.Getpid()) {
		t.Error("own pid should be alive")
	}
	if pidAlive(0) || pidAlive(-1) {
		t.Error("non-positive pids are never alive")
	}
	// 0x7FFFFFFE is almost certainly not a live pid.
	if pidAlive(0x7FFFFFFE) {
		t.Skip("unexpectedly-live high pid; skip")
	}
}

func TestRecapForLiveSession(t *testing.T) {
	// Lay out a fake ~/.claude/projects transcript and resolve it by cwd.
	home := t.TempDir()
	cwd := "/Users/p/projects/domux"
	projDir := filepath.Join(home, ".claude", "projects", claudeProjectDirName(cwd))
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		t.Fatal(err)
	}
	writeTranscript(t, filepath.Join(projDir, "old.jsonl"), "Old title")
	writeTranscript(t, filepath.Join(projDir, "new.jsonl"), "New title")

	t.Setenv("HOME", home)

	sessions := []claudeSession{
		{SessionID: "old", Cwd: cwd, Pid: os.Getpid(), UpdatedAt: 100},
		{SessionID: "new", Cwd: cwd, Pid: os.Getpid(), UpdatedAt: 200}, // most recent wins
		{SessionID: "other", Cwd: "/somewhere/else", Pid: os.Getpid(), UpdatedAt: 999},
	}

	if got := recapForLiveSession(sessions, cwd); got != "New title" {
		t.Errorf("recapForLiveSession(cwd) = %q, want %q", got, "New title")
	}
	if got := recapForLiveSession(sessions, "/no/match"); got != "" {
		t.Errorf("recapForLiveSession(no match) = %q, want empty", got)
	}
	if got := recapForLiveSession(nil, cwd); got != "" {
		t.Errorf("recapForLiveSession(no sessions) = %q, want empty", got)
	}
}

func TestLastAITitle(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "s.jsonl")
	// big line to ensure we don't choke on long transcript entries, an early
	// title, then a later title that should win.
	big := `{"type":"assistant","content":"` + string(make([]byte, 300*1024)) + `"}`
	content := `{"type":"user","content":"hi"}` + "\n" +
		`{"type":"ai-title","aiTitle":"First title","sessionId":"x"}` + "\n" +
		big + "\n" +
		`{"type":"ai-title","aiTitle":"Final title","sessionId":"x"}` + "\n" +
		`{"type":"ai-title","aiTitle":"","sessionId":"x"}` + "\n"
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := lastAITitle(path); got != "Final title" {
		t.Errorf("lastAITitle = %q, want %q", got, "Final title")
	}
}

func TestLastAITitleNone(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "s.jsonl")
	if err := os.WriteFile(path, []byte(`{"type":"user","content":"hi"}`+"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := lastAITitle(path); got != "" {
		t.Errorf("lastAITitle = %q, want empty", got)
	}
	if got := lastAITitle(filepath.Join(dir, "missing.jsonl")); got != "" {
		t.Errorf("lastAITitle(missing) = %q, want empty", got)
	}
}

func TestScanRecapPrefersAwaySummary(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "s.jsonl")
	lines := []string{
		`{"type":"ai-title","aiTitle":"Stale title"}`,
		`{"type":"system","subtype":"away_summary","content":"Goal: first recap. Done: x."}`,
		// a later auto-recap supersedes the earlier one
		`{"type":"system","subtype":"away_summary","content":"Building the latest recap line. Next: ship."}`,
	}
	writeLines(t, path, lines)

	summary, title := scanRecap(path)
	if summary != "Building the latest recap line" {
		t.Errorf("summary = %q, want first sentence of latest away_summary", summary)
	}
	if title != "Stale title" {
		t.Errorf("title = %q, want %q", title, "Stale title")
	}
	if got := cachedRecap(path); got != "Building the latest recap line" {
		t.Errorf("cachedRecap = %q, want summary to win over title", got)
	}
}

func TestScanRecapManualRecapAndPairing(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "s.jsonl")
	// /recap stdout is captured; a later /name command's stdout is NOT.
	writeLines(t, path, []string{
		`{"type":"ai-title","aiTitle":"Title"}`,
		`{"type":"user","message":{"role":"user","content":"<command-name>/recap</command-name>"}}`,
		`{"type":"system","subtype":"local_command","content":"<local-command-stdout>Manual recap. Extra detail.</local-command-stdout>"}`,
		`{"type":"user","message":{"role":"user","content":"<command-name>/name</command-name>"}}`,
		`{"type":"system","subtype":"local_command","content":"<local-command-stdout>renamed session</local-command-stdout>"}`,
	})
	if got := cachedRecap(path); got != "Manual recap" {
		t.Errorf("cachedRecap = %q, want %q (recap paired + first sentence, /name stdout ignored)", got, "Manual recap")
	}
}

func TestScanRecapIgnoresQuotedMarkers(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "s.jsonl")
	// An assistant message that merely *quotes* the markers must not be parsed
	// as a recap (JSON-typed dispatch: content is an array, type is assistant).
	writeLines(t, path, []string{
		`{"type":"ai-title","aiTitle":"Real title"}`,
		`{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"the away_summary subtype and <command-name>/recap</command-name> and <local-command-stdout>fake</local-command-stdout>"}]}}`,
	})
	if got := cachedRecap(path); got != "Real title" {
		t.Errorf("cachedRecap = %q, want %q (quoted markers ignored)", got, "Real title")
	}
}

func TestStdoutInner(t *testing.T) {
	if got := stdoutInner("<local-command-stdout>raw  text</local-command-stdout>"); got != "raw  text" {
		t.Errorf("stdoutInner = %q, want raw inner text", got)
	}
	if got := stdoutInner("no markers here"); got != "" {
		t.Errorf("stdoutInner(no markers) = %q, want empty", got)
	}
}

func TestRecapLine(t *testing.T) {
	cases := map[string]string{
		"Goal: show each recap. Done: x. Next: y": "show each recap",
		"Babysitting PR #66 (gen). Fixed bugs":    "Babysitting PR #66 (gen)",
		"single sentence no period":               "single sentence no period",
		"One sentence ending in a period.":        "One sentence ending in a period",
		"  Goal:  collapse   whitespace.  more ":  "collapse whitespace",
	}
	for in, want := range cases {
		if got := recapLine(in); got != want {
			t.Errorf("recapLine(%q) = %q, want %q", in, got, want)
		}
	}
	// No length cap: a long clause is returned whole (the picker wraps it).
	long := "Goal: " + strings.Repeat("x", 200)
	if got := recapLine(long); got != strings.Repeat("x", 200) {
		t.Errorf("recapLine(long) = %q, want full uncapped clause", got)
	}
}

func writeLines(t *testing.T, path string, lines []string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(strings.Join(lines, "\n")+"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
}

func writeTranscript(t *testing.T, path, title string) {
	t.Helper()
	line := `{"type":"ai-title","aiTitle":"` + title + `","sessionId":"x"}` + "\n"
	if err := os.WriteFile(path, []byte(line), 0o644); err != nil {
		t.Fatal(err)
	}
}
