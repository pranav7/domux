package main

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
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

func TestBestLiveSession(t *testing.T) {
	cwd := "/Users/p/projects/domux"
	sessions := []claudeSession{
		{SessionID: "old", Cwd: cwd, Pid: os.Getpid(), UpdatedAt: 100},
		{SessionID: "new", Cwd: cwd, Pid: os.Getpid(), UpdatedAt: 200}, // most recent wins
		{SessionID: "other", Cwd: "/elsewhere", Pid: os.Getpid(), UpdatedAt: 999},
	}
	if s, ok := bestLiveSession(sessions, cwd); !ok || s.SessionID != "new" {
		t.Errorf("bestLiveSession(cwd) = %q,%v want new,true", s.SessionID, ok)
	}
	if _, ok := bestLiveSession(sessions, "/no/match"); ok {
		t.Error("bestLiveSession(no match) should be false")
	}
	if _, ok := bestLiveSession(nil, cwd); ok {
		t.Error("bestLiveSession(nil) should be false")
	}
	if _, ok := bestLiveSession(sessions); ok {
		t.Error("bestLiveSession(no paths) should be false")
	}
}

func TestBestLiveSessionByTTY(t *testing.T) {
	cwd := "/Users/p/projects/audrey"
	// Three live Claude sessions sharing one cwd — the exact shape that made
	// every window row show the same recap when matched by cwd alone.
	sessions := []claudeSession{
		{SessionID: "w1", Cwd: cwd, Pid: 1, UpdatedAt: 100, TTY: "ttys001"},
		{SessionID: "w2", Cwd: cwd, Pid: 2, UpdatedAt: 200, TTY: "ttys011"},
		{SessionID: "detached", Cwd: cwd, Pid: 3, UpdatedAt: 300, TTY: ""},
	}
	// Each pane's tty resolves to its OWN session despite the shared cwd. tmux's
	// "/dev/" prefix is stripped before comparison.
	if s, ok := bestLiveSessionByTTY(sessions, "/dev/ttys001"); !ok || s.SessionID != "w1" {
		t.Errorf("ttys001 => %q,%v want w1,true", s.SessionID, ok)
	}
	if s, ok := bestLiveSessionByTTY(sessions, "/dev/ttys011"); !ok || s.SessionID != "w2" {
		t.Errorf("ttys011 => %q,%v want w2,true", s.SessionID, ok)
	}
	if _, ok := bestLiveSessionByTTY(sessions, "/dev/ttys999"); ok {
		t.Error("unmatched tty should not resolve")
	}
	// Empty ttys on either side must never match — else the detached session
	// (TTY "") would be handed to any pane whose tty failed to resolve.
	if _, ok := bestLiveSessionByTTY(sessions, "", "/dev/"); ok {
		t.Error("empty ttys should not resolve to the detached session")
	}
	if _, ok := bestLiveSessionByTTY(nil, "/dev/ttys001"); ok {
		t.Error("no sessions should not resolve")
	}
	// Two sessions sharing a tty (e.g. a stale registry entry) → most recent wins.
	dup := []claudeSession{
		{SessionID: "stale", Cwd: cwd, Pid: 4, UpdatedAt: 50, TTY: "ttys002"},
		{SessionID: "fresh", Cwd: cwd, Pid: 5, UpdatedAt: 500, TTY: "ttys002"},
	}
	if s, ok := bestLiveSessionByTTY(dup, "ttys002"); !ok || s.SessionID != "fresh" {
		t.Errorf("shared tty => %q,%v want fresh,true", s.SessionID, ok)
	}
}

func TestParsePsTTYLines(t *testing.T) {
	// "??" (macOS) and "?" (Linux) both mean "no controlling terminal".
	out := "  49115 ttys001\n  56103 ttys011\n  78813 ??\n  333 ?\n  700 pts/3\nbogus line\n  12 \n"
	got := parsePsTTYLines(out)
	if got[49115] != "ttys001" || got[56103] != "ttys011" {
		t.Errorf("parsePsTTYLines = %+v, want ttys001/ttys011 for 49115/56103", got)
	}
	if got[700] != "pts/3" {
		t.Errorf("linux pts tty: got[700] = %q, want pts/3", got[700])
	}
	if _, ok := got[78813]; ok {
		t.Errorf("pid with ?? (no controlling tty) should be omitted, got %+v", got)
	}
	if _, ok := got[333]; ok {
		t.Errorf("pid with ? (linux, no controlling tty) should be omitted, got %+v", got)
	}
	if _, ok := got[12]; ok {
		t.Errorf("pid with empty tty should be omitted, got %+v", got)
	}
}

func TestScanRecapReturnsTimestamp(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "s.jsonl")
	writeLines(t, path, []string{
		`{"type":"system","subtype":"away_summary","content":"Old recap.","timestamp":"2026-05-21T10:00:00.000Z"}`,
		`{"type":"system","subtype":"away_summary","content":"New recap.","timestamp":"2026-05-21T12:30:00.500Z"}`,
	})
	summary, ts, _ := scanRecap(path)
	if summary != "New recap" {
		t.Fatalf("summary = %q, want New recap", summary)
	}
	want, _ := time.Parse(time.RFC3339, "2026-05-21T12:30:00.500Z")
	if !ts.Equal(want) {
		t.Errorf("summaryTime = %v, want %v (timestamp of freshest entry)", ts, want)
	}
	// ai-title fallback carries no timestamp → zero recapTime.
	titlePath := filepath.Join(dir, "t.jsonl")
	writeTranscript(t, titlePath, "Just a title")
	if recap, rt := cachedRecap(titlePath); recap != "Just a title" || !rt.IsZero() {
		t.Errorf("cachedRecap(title) = %q,%v want title with zero time", recap, rt)
	}
}

func TestRecapVisibleAfterClear(t *testing.T) {
	cleared := "2026-05-21T11:00:00Z"
	before, _ := time.Parse(time.RFC3339, "2026-05-21T10:00:00Z")
	after, _ := time.Parse(time.RFC3339, "2026-05-21T12:00:00Z")

	if !recapVisibleAfterClear(before, "") {
		t.Error("never-cleared workspace should always show its recap")
	}
	if recapVisibleAfterClear(before, cleared) {
		t.Error("recap dated before clear should be hidden")
	}
	if recapVisibleAfterClear(time.Time{}, cleared) {
		t.Error("ai-title fallback (zero time) should be hidden after a clear")
	}
	if !recapVisibleAfterClear(after, cleared) {
		t.Error("recap written after clear should resurface")
	}
	if !recapVisibleAfterClear(before, "not-a-time") {
		t.Error("unparseable clear time should not suppress")
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

	summary, _, title := scanRecap(path)
	if summary != "Building the latest recap line" {
		t.Errorf("summary = %q, want first sentence of latest away_summary", summary)
	}
	if title != "Stale title" {
		t.Errorf("title = %q, want %q", title, "Stale title")
	}
	if got, _ := cachedRecap(path); got != "Building the latest recap line" {
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
	if got, _ := cachedRecap(path); got != "Manual recap" {
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
	if got, _ := cachedRecap(path); got != "Real title" {
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

func TestReadClaudeSessionsForResumeIncludesDeadPids(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	dir := filepath.Join(home, ".claude", "sessions")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	// A dead pid (0x7FFFFFFE) — readClaudeSessions would drop it, but resume must keep it.
	rec := `{"pid":2147483646,"sessionId":"dead-sess","cwd":"/p/domux","updatedAt":1700}`
	if err := os.WriteFile(filepath.Join(dir, "2147483646.json"), []byte(rec), 0o644); err != nil {
		t.Fatal(err)
	}

	got := readClaudeSessionsForResume()
	if len(got) != 1 || got[0].SessionID != "dead-sess" || got[0].Cwd != "/p/domux" || got[0].UpdatedAt != 1700 {
		t.Fatalf("readClaudeSessionsForResume() = %+v, want one dead-sess entry", got)
	}
	if got[0].Agent != "claude" {
		t.Fatalf("agent = %q, want claude", got[0].Agent)
	}
}

func TestReadCodexSessions(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	day := filepath.Join(home, ".codex", "sessions", "2026", "07", "13")
	if err := os.MkdirAll(day, 0o755); err != nil {
		t.Fatal(err)
	}
	top := `{"timestamp":"t","type":"session_meta","payload":{"id":"top-uuid","cwd":"/p/audrey","thread_source":"user"}}` + "\n{\"type\":\"message\"}\n"
	sub := `{"timestamp":"t","type":"session_meta","payload":{"id":"sub-uuid","cwd":"/p/audrey","thread_source":"subagent"}}` + "\n"
	if err := os.WriteFile(filepath.Join(day, "rollout-top.jsonl"), []byte(top), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(day, "rollout-sub.jsonl"), []byte(sub), 0o644); err != nil {
		t.Fatal(err)
	}
	// Malformed file must not crash the reader.
	if err := os.WriteFile(filepath.Join(day, "rollout-bad.jsonl"), []byte("not json\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	got := readCodexSessions()
	if len(got) != 1 {
		t.Fatalf("readCodexSessions() = %+v, want exactly the top-level session", got)
	}
	if got[0].SessionID != "top-uuid" || got[0].Cwd != "/p/audrey" || got[0].Agent != "codex" {
		t.Fatalf("codex session = %+v", got[0])
	}
	if got[0].UpdatedAt == 0 {
		t.Fatalf("codex UpdatedAt should be the file mtime, got 0")
	}
}

func TestReadOpencodeSessions(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "call")
	installFakeOpencode(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_OPENCODE_CALL"
cat <<'JSON'
[{"id":"ses_1","title":"a","updated":1783952921264,"directory":"/p/domux"},
 {"id":"ses_2","title":"b","updated":1783952584360,"directory":"/p/audrey"}]
JSON
`, callFile)

	got := readOpencodeSessions()
	if len(got) != 2 {
		t.Fatalf("readOpencodeSessions() = %+v, want 2", got)
	}
	if got[0].SessionID != "ses_1" || got[0].Cwd != "/p/domux" || got[0].UpdatedAt != 1783952921264 || got[0].Agent != "opencode" {
		t.Fatalf("opencode session[0] = %+v", got[0])
	}
}

func TestReadOpencodeSessionsMissingBinaryReturnsNil(t *testing.T) {
	// Point PATH at an empty dir so `opencode` is not found.
	t.Setenv("PATH", t.TempDir())
	if got := readOpencodeSessions(); got != nil {
		t.Fatalf("missing opencode should yield nil, got %+v", got)
	}
}

func TestBestAgentSession(t *testing.T) {
	// Sorted UpdatedAt-descending, as readAgentSessions guarantees.
	sessions := []agentSession{
		{Agent: "codex", SessionID: "cx", Cwd: "/p/domux", UpdatedAt: 300},
		{Agent: "claude", SessionID: "cl", Cwd: "/p/domux", UpdatedAt: 200},
		{Agent: "opencode", SessionID: "oc", Cwd: "/p/audrey", UpdatedAt: 100},
	}
	// Two agents share /p/domux; the more-recent (codex, 300) wins.
	if s, ok := bestAgentSession(sessions, "/p/domux"); !ok || s.SessionID != "cx" {
		t.Fatalf("bestAgentSession(/p/domux) = %+v,%v want cx,true", s, ok)
	}
	if s, ok := bestAgentSession(sessions, "/p/audrey"); !ok || s.Agent != "opencode" {
		t.Fatalf("bestAgentSession(/p/audrey) = %+v,%v want opencode,true", s, ok)
	}
	if _, ok := bestAgentSession(sessions, "/no/match"); ok {
		t.Fatal("no-match should be false")
	}
	if _, ok := bestAgentSession(sessions); ok {
		t.Fatal("no-paths should be false")
	}
}

func TestReadAgentSessionsSortsMostRecentFirst(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	// Only Claude here (no codex dir, opencode shimmed to nothing) — enough to
	// confirm the unified reader returns claude entries and doesn't crash on
	// missing codex/opencode.
	t.Setenv("PATH", t.TempDir()) // opencode not found → nil
	dir := filepath.Join(home, ".claude", "sessions")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	writeClaudeSessionFile(t, dir, "1", "older", "/p/a", 100)
	writeClaudeSessionFile(t, dir, "2", "newer", "/p/b", 500)

	got := readAgentSessions()
	if len(got) != 2 {
		t.Fatalf("readAgentSessions() = %+v, want 2", got)
	}
	if got[0].SessionID != "newer" || got[1].SessionID != "older" {
		t.Fatalf("not sorted most-recent-first: %+v", got)
	}
}

func writeClaudeSessionFile(t *testing.T, dir, pid, sid, cwd string, updated int64) {
	t.Helper()
	rec := `{"pid":` + pid + `,"sessionId":"` + sid + `","cwd":"` + cwd + `","updatedAt":` + strconv.FormatInt(updated, 10) + `}`
	if err := os.WriteFile(filepath.Join(dir, pid+".json"), []byte(rec), 0o644); err != nil {
		t.Fatal(err)
	}
}

func installFakeOpencode(t *testing.T, script, callFile string) {
	t.Helper()
	binDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(binDir, "opencode"), []byte(script), 0o755); err != nil {
		t.Fatalf("write fake opencode: %v", err)
	}
	t.Setenv("DOMUX_OPENCODE_CALL", callFile)
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
}
