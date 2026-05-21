package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestClearSessionStateFilesRemovesLegacySessionMetadata(t *testing.T) {
	homeDir := t.TempDir()
	session := "audrey-app"
	files := []string{
		".tmux-label-" + session,
		".tmux-server-" + session,
		".tmux-workspace-" + session,
		".tmux-pr-" + session,
		".tmux-claude-" + session,
		".tmux-claude-" + session + "_0_0",
		".tmux-codex-" + session,
		".tmux-codex-" + session + "_0_1",
	}
	for _, name := range files {
		if err := os.WriteFile(filepath.Join(homeDir, name), []byte("value\n"), 0644); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}
	unrelated := ".tmux-pr-other-session"
	if err := os.WriteFile(filepath.Join(homeDir, unrelated), []byte("value\n"), 0644); err != nil {
		t.Fatalf("write unrelated file: %v", err)
	}

	if err := clearSessionStateFiles(homeDir, session); err != nil {
		t.Fatalf("clearSessionStateFiles: %v", err)
	}

	for _, name := range files {
		if _, err := os.Stat(filepath.Join(homeDir, name)); !os.IsNotExist(err) {
			t.Fatalf("expected %s to be removed, stat err = %v", name, err)
		}
	}
	if _, err := os.Stat(filepath.Join(homeDir, unrelated)); err != nil {
		t.Fatalf("expected unrelated file to remain: %v", err)
	}
}

func TestAggregateAIStatesKeepsClaudeAndCodexSeparate(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"claude:0_0": "WAITING",
		"claude:0_1": "CLAUDING",
		"codex:0_2":  "CODEXING",
		"codex:0_3":  "WAITING",
	}}

	got := aggregateAIStatesFromSession(state)

	if got.Claude != "WAITING" {
		t.Fatalf("Claude = %q", got.Claude)
	}
	if got.Codex != "WAITING" {
		t.Fatalf("Codex = %q", got.Codex)
	}
}

func TestAggregateAIStatesTreatsLegacyAIAsClaude(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"0_0": "CLAUDING",
	}}

	got := aggregateAIStatesFromSession(state)

	if got.Claude != "CLAUDING" {
		t.Fatalf("Claude = %q", got.Claude)
	}
	if got.Codex != "" {
		t.Fatalf("Codex = %q", got.Codex)
	}
}

func TestTmuxAIBadgesUseAgentWaitingLabels(t *testing.T) {
	if got := tmuxAIBadge("claude", "WAITING"); !strings.Contains(got, "CLAUDE WAITING") {
		t.Fatalf("Claude badge = %q", got)
	}
	if got := tmuxAIBadge("codex", "WAITING"); !strings.Contains(got, "CODEX WAITING") {
		t.Fatalf("Codex badge = %q", got)
	}
}

func TestSingleMatchingStartSessionAttachesDirectly(t *testing.T) {
	session, ok := singleMatchingStartSession([]string{"dotfiles"})
	if !ok {
		t.Fatalf("expected single match to attach directly")
	}
	if session != "dotfiles" {
		t.Fatalf("session = %q", session)
	}
}

func TestMultipleMatchingStartSessionsUsePicker(t *testing.T) {
	_, ok := singleMatchingStartSession([]string{"dotfiles", "dotfiles-2"})
	if ok {
		t.Fatalf("expected multiple matches to use picker")
	}
}

func TestTmuxDisplayArgsTargetsEnvPane(t *testing.T) {
	t.Setenv("TMUX_PANE", "%12")

	got := tmuxDisplayArgs("#S")
	want := []string{"display-message", "-t", "%12", "-p", "#S"}
	if strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("args = %#v", got)
	}
}

func TestTmuxDisplayArgsFallsBackToActivePane(t *testing.T) {
	t.Setenv("TMUX_PANE", "")

	got := tmuxDisplayArgs("#S")
	want := []string{"display-message", "-p", "#S"}
	if strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("args = %#v", got)
	}
}

func TestAttachTmuxSessionUsesAttachWithoutTmuxEnv(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	t.Setenv("TMUX", "")
	t.Setenv("TMUX_PANE", "%12")
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
case "$1" in
display-message)
	echo dotfiles
	exit 0
	;;
attach-session)
	printf 'attach %s %s\n' "$2" "$3" > "$DOMUX_TMUX_CALL"
	exit 0
	;;
switch-client)
	printf 'switch %s %s\n' "$2" "$3" > "$DOMUX_TMUX_CALL"
	echo 'no current client' >&2
	exit 1
	;;
esac
exit 2
`, callFile)

	if err := attachTmuxSession("audrey"); err != nil {
		t.Fatalf("attachTmuxSession: %v", err)
	}

	data, err := os.ReadFile(callFile)
	if err != nil {
		t.Fatalf("read tmux call: %v", err)
	}
	if got := string(data); got != "attach -t audrey\n" {
		t.Fatalf("tmux call = %q", got)
	}
}

func TestRefreshTmuxClientIgnoresNoCurrentClient(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" > "$DOMUX_TMUX_CALL"
echo 'no current client' >&2
exit 1
`, callFile)

	if err := refreshTmuxClient(); err != nil {
		t.Fatalf("refreshTmuxClient: %v", err)
	}
}

func installFakeTmux(t *testing.T, script, callFile string) {
	t.Helper()
	binDir := t.TempDir()
	path := filepath.Join(binDir, "tmux")
	if err := os.WriteFile(path, []byte(script), 0755); err != nil {
		t.Fatalf("write fake tmux: %v", err)
	}
	t.Setenv("DOMUX_TMUX_CALL", callFile)
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
}

func TestMergeLegacyStateIncludesFreshLegacyAIWithPaneState(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "dotfiles"
	if err := os.WriteFile(filepath.Join(homeDir, ".tmux-claude-"+session), []byte("CLAUDING\n"), 0644); err != nil {
		t.Fatalf("write legacy claude state: %v", err)
	}
	if err := os.WriteFile(filepath.Join(homeDir, ".tmux-claude-"+session+"_1_0"), []byte("IDLE\n"), 0644); err != nil {
		t.Fatalf("write pane claude state: %v", err)
	}

	state := &SessionState{Name: session, AI: map[string]string{}}
	mergeLegacyState(state)
	got := aggregateAIStatesFromSession(state)

	if got.Claude != "CLAUDING" {
		t.Fatalf("Claude = %q, AI = %#v", got.Claude, state.AI)
	}
}

func TestMergeLegacyStateIgnoresStaleClaudingFile(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "workspace-1"
	path := filepath.Join(homeDir, ".tmux-claude-"+session)
	if err := os.WriteFile(path, []byte("CLAUDING\n"), 0644); err != nil {
		t.Fatalf("write legacy claude state: %v", err)
	}
	stale := time.Now().Add(-2 * time.Minute)
	if err := os.Chtimes(path, stale, stale); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	state := &SessionState{Name: session, AI: map[string]string{}}
	mergeLegacyState(state)
	got := aggregateAIStatesFromSession(state)

	if got.Claude == "CLAUDING" {
		t.Fatalf("stale CLAUDING leaked through: AI = %#v", state.AI)
	}
}

func TestMergeLegacyStateKeepsStaleWaitingFile(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "workspace-1"
	path := filepath.Join(homeDir, ".tmux-claude-"+session)
	if err := os.WriteFile(path, []byte("WAITING\n"), 0644); err != nil {
		t.Fatalf("write legacy claude state: %v", err)
	}
	old := time.Now().Add(-10 * time.Minute)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	state := &SessionState{Name: session, AI: map[string]string{}}
	mergeLegacyState(state)
	got := aggregateAIStatesFromSession(state)

	if got.Claude != "WAITING" {
		t.Fatalf("WAITING dropped: AI = %#v", state.AI)
	}
}

func TestSaveSessionStateDropsLegacyPseudoPane(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audrey-app"
	state := &SessionState{
		Name: session,
		AI: map[string]string{
			"claude:legacy": "CLAUDING",
			"claude:1_0":    "CLAUDING",
		},
	}
	if err := saveSessionState(state); err != nil {
		t.Fatalf("saveSessionState: %v", err)
	}
	path, err := sessionStatePath(session)
	if err != nil {
		t.Fatalf("sessionStatePath: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read state: %v", err)
	}
	if strings.Contains(string(data), "claude:legacy") {
		t.Fatalf("legacy pseudo-pane leaked into persisted state: %s", data)
	}
	if !strings.Contains(string(data), "claude:1_0") {
		t.Fatalf("real pane key dropped: %s", data)
	}
}

func TestAIStateAllClearRemovesAllAgentPanes(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "dotfiles"
	state := SessionState{
		Name: session,
		AI: map[string]string{
			"codex:1_0":  "CODEXING",
			"codex:1_1":  "CODEXING",
			"claude:1_0": "CLAUDING",
		},
	}
	if err := saveSessionState(&state); err != nil {
		t.Fatalf("saveSessionState: %v", err)
	}
	for _, pane := range []string{"1_0", "1_1"} {
		name := ".tmux-codex-" + session + "_" + pane
		if err := os.WriteFile(filepath.Join(homeDir, name), []byte("CODEXING\n"), 0644); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}

	if err := aiStateCommand([]string{"--session", session, "--agent", "codex", "--all", "clear"}); err != nil {
		t.Fatalf("aiStateCommand: %v", err)
	}

	got := loadSessionStateWithLegacy(session)
	if _, ok := got.AI["codex:1_0"]; ok {
		t.Fatalf("codex:1_0 remains: %#v", got.AI)
	}
	if _, ok := got.AI["codex:1_1"]; ok {
		t.Fatalf("codex:1_1 remains: %#v", got.AI)
	}
	if got.AI["claude:1_0"] != "CLAUDING" {
		t.Fatalf("claude state removed: %#v", got.AI)
	}
	for _, pane := range []string{"1_0", "1_1"} {
		name := ".tmux-codex-" + session + "_" + pane
		if _, err := os.Stat(filepath.Join(homeDir, name)); !os.IsNotExist(err) {
			t.Fatalf("expected %s removed, stat err = %v", name, err)
		}
	}
}
