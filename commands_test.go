package main

import (
	"os"
	"path/filepath"
	"strings"
	"sync"
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

func TestClearWorkspaceDirtyKeepsSessionState(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audrey-app"
	state := &SessionState{
		Name:   session,
		Root:   root,
		Label:  "PBC",
		Server: true,
		AI:     map[string]string{"codex:0_0": "CODEXING"},
	}
	if err := saveSessionState(state); err != nil {
		t.Fatalf("saveSessionState: %v", err)
	}
	legacyLabel := filepath.Join(homeDir, ".tmux-label-"+session)
	if err := os.WriteFile(legacyLabel, []byte("PBC\n"), 0644); err != nil {
		t.Fatalf("write legacy label: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "dirty.txt"), []byte("x"), 0644); err != nil {
		t.Fatalf("write dirty: %v", err)
	}

	err := clearWorkspaceForSession(session, root, false)
	if err != errClearDirty {
		t.Fatalf("err = %v, want errClearDirty", err)
	}
	got := loadSessionStateWithLegacy(session)
	if got.Label != "PBC" || !got.Server || got.AI["codex:0_0"] != "CODEXING" {
		t.Fatalf("state changed after dirty clear: %#v", got)
	}
	if _, err := os.Stat(legacyLabel); err != nil {
		t.Fatalf("legacy label removed after dirty clear: %v", err)
	}
}

func TestClearWorkspaceClearsSessionTodos(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audrey-app"
	todoPath := filepath.Join(t.TempDir(), "todo.md")
	list := &List{
		Worktree: "/tmp/audrey-app",
		Created:  "2026-05-28",
		Active:   []Item{{Title: "open"}, {Title: "done", Done: true}},
		Archive:  []Item{{Title: "old", Done: true}},
	}
	if err := saveList(todoPath, list); err != nil {
		t.Fatalf("saveList: %v", err)
	}
	if err := saveSessionState(&SessionState{Name: session, TodoPath: todoPath}); err != nil {
		t.Fatalf("saveSessionState: %v", err)
	}

	if err := clearWorkspaceForSession(session, "", false); err != nil {
		t.Fatalf("clearWorkspaceForSession: %v", err)
	}

	got, err := loadList(todoPath)
	if err != nil {
		t.Fatalf("loadList: %v", err)
	}
	if len(got.Active) != 0 || len(got.Archive) != 0 {
		t.Fatalf("todos not cleared: active=%#v archive=%#v", got.Active, got.Archive)
	}
}

func TestCloseTmuxSessionClearsStateAndKillsSession(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audreyai_azure_tf_internal"
	state := &SessionState{
		Name:   session,
		Root:   "/tmp/audreyai",
		Label:  "done",
		Server: true,
		AI:     map[string]string{"codex:0_0": "CODEXING"},
	}
	if err := saveSessionState(state); err != nil {
		t.Fatalf("saveSessionState: %v", err)
	}
	legacy := []string{
		".tmux-label-" + session,
		".tmux-server-" + session,
		".tmux-codex-" + session + "_0_0",
	}
	for _, name := range legacy {
		if err := os.WriteFile(filepath.Join(homeDir, name), []byte("x\n"), 0644); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session|kill-session|refresh-client) exit 0 ;;
*) exit 1 ;;
esac
`, callFile)

	if err := closeTmuxSession(session); err != nil {
		t.Fatalf("closeTmuxSession: %v", err)
	}

	statePath, err := sessionStatePath(session)
	if err != nil {
		t.Fatalf("sessionStatePath: %v", err)
	}
	if _, err := os.Stat(statePath); !os.IsNotExist(err) {
		t.Fatalf("state file still exists, stat err = %v", err)
	}
	for _, name := range legacy {
		if _, err := os.Stat(filepath.Join(homeDir, name)); !os.IsNotExist(err) {
			t.Fatalf("legacy %s still exists, stat err = %v", name, err)
		}
	}
	data, err := os.ReadFile(callFile)
	if err != nil {
		t.Fatalf("read tmux call: %v", err)
	}
	if !strings.Contains(string(data), "kill-session -t "+session) {
		t.Fatalf("tmux calls = %q", data)
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

func TestAggregateAIStatesKeepsAgentLabelsSeparate(t *testing.T) {
	state := &SessionState{
		AI: map[string]string{
			"claude:0_1": "CLAUDING",
			"codex:0_2":  "CODEXING",
		},
		AIWorkingLabels: map[string]string{
			"claude": "Pondering",
			"codex":  "Computing",
		},
	}

	got := aggregateAIStatesFromSession(state)

	if got.ClaudeLabel != "Pondering" {
		t.Fatalf("Claude label = %q", got.ClaudeLabel)
	}
	if got.CodexLabel != "Computing" {
		t.Fatalf("Codex label = %q", got.CodexLabel)
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

func TestTmuxAIBadgeRendersCompacting(t *testing.T) {
	got := tmuxAIBadge("claude", "COMPACTING")
	if !strings.Contains(got, "Compacting") {
		t.Fatalf("Claude compacting badge = %q", got)
	}
	if !strings.Contains(got, "✦") {
		t.Fatalf("Claude compacting badge missing star glyph: %q", got)
	}
	if !strings.Contains(got, "#AFAFFF") {
		t.Fatalf("Claude compacting badge missing compact purple: %q", got)
	}
}

func TestCompactingOutranksClauding(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"claude:0_0": "CLAUDING",
		"claude:0_1": "COMPACTING",
	}}
	got := aggregateAIStatesFromSession(state)
	if got.Claude != "COMPACTING" {
		t.Fatalf("Claude = %q, want COMPACTING", got.Claude)
	}
}

func TestWaitingOutranksCompacting(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"claude:0_0": "COMPACTING",
		"claude:0_1": "WAITING",
	}}
	got := aggregateAIStatesFromSession(state)
	if got.Claude != "WAITING" {
		t.Fatalf("Claude = %q, want WAITING", got.Claude)
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

func TestMergeLegacyStatePrunesOrphanedJSONClauding(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "workspace-1"

	// Fresh pane 1_0 — backing legacy file written just now.
	fresh := filepath.Join(homeDir, ".tmux-claude-"+session+"_1_0")
	if err := os.WriteFile(fresh, []byte("CLAUDING\n"), 0644); err != nil {
		t.Fatalf("write fresh pane file: %v", err)
	}

	// Stale pane 1_2 — JSON entry exists but backing legacy file is 3 days old.
	stalePath := filepath.Join(homeDir, ".tmux-claude-"+session+"_1_2")
	if err := os.WriteFile(stalePath, []byte("CLAUDING\n"), 0644); err != nil {
		t.Fatalf("write stale pane file: %v", err)
	}
	old := time.Now().Add(-72 * time.Hour)
	if err := os.Chtimes(stalePath, old, old); err != nil {
		t.Fatalf("chtimes stale: %v", err)
	}

	state := &SessionState{
		Name: session,
		AI: map[string]string{
			"claude:1_0": "CLAUDING",
			"claude:1_2": "CLAUDING",
		},
	}
	mergeLegacyState(state)

	if _, ok := state.AI["claude:1_0"]; !ok {
		t.Fatalf("fresh pane 1_0 pruned: AI = %#v", state.AI)
	}
	if _, ok := state.AI["claude:1_2"]; ok {
		t.Fatalf("stale pane 1_2 not pruned: AI = %#v", state.AI)
	}
}

func TestMergeLegacyStatePrunesMissingLegacyCodex(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audrey-app"
	old := time.Now().Add(-2 * time.Minute).Format(timeFormat)
	state := &SessionState{
		Name:      session,
		UpdatedAt: old,
		AI: map[string]string{
			"codex:1_1": "CODEXING",
		},
	}

	mergeLegacyState(state)

	if _, ok := state.AI["codex:1_1"]; ok {
		t.Fatalf("missing legacy codex state not pruned: AI = %#v", state.AI)
	}
}

func TestMergeLegacyStateKeepsRecentMissingLegacyCodex(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audrey-app"
	state := &SessionState{
		Name:      session,
		UpdatedAt: time.Now().Format(timeFormat),
		AI: map[string]string{
			"codex:1_1": "CODEXING",
		},
	}

	mergeLegacyState(state)

	if _, ok := state.AI["codex:1_1"]; !ok {
		t.Fatalf("recent missing legacy codex state pruned: AI = %#v", state.AI)
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

func TestSaveSessionStateUsesUniqueTempFiles(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "dotfiles"
	values := []string{"CODEXING", "WAITING", "IDLE", "CODEXING"}

	var wg sync.WaitGroup
	errs := make(chan error, len(values))
	for _, value := range values {
		value := value
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs <- saveSessionState(&SessionState{
				Name: session,
				AI: map[string]string{
					"codex:1_0": value,
				},
			})
		}()
	}
	wg.Wait()
	close(errs)

	for err := range errs {
		if err != nil {
			t.Fatalf("saveSessionState: %v", err)
		}
	}
	path, err := sessionStatePath(session)
	if err != nil {
		t.Fatalf("sessionStatePath: %v", err)
	}
	matches, err := filepath.Glob(path + ".*.tmp")
	if err != nil {
		t.Fatalf("glob tmp files: %v", err)
	}
	if len(matches) != 0 {
		t.Fatalf("temp files left behind: %#v", matches)
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

func TestAIStateAllClearPersistsPrunedMissingLegacyState(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "audrey-app"
	path, err := sessionStatePath(session)
	if err != nil {
		t.Fatalf("sessionStatePath: %v", err)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		t.Fatalf("mkdir state dir: %v", err)
	}
	old := time.Now().Add(-2 * time.Minute).Format(timeFormat)
	data := `{
  "name": "audrey-app",
  "ai": {
    "codex:1_1": "CODEXING"
  },
  "updated_at": "` + old + `"
}
`
	if err := os.WriteFile(path, []byte(data), 0644); err != nil {
		t.Fatalf("write state: %v", err)
	}

	if err := aiStateCommand([]string{"--session", session, "--agent", "codex", "--all", "clear"}); err != nil {
		t.Fatalf("aiStateCommand: %v", err)
	}

	gotData, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read state: %v", err)
	}
	if strings.Contains(string(gotData), "codex:1_1") {
		t.Fatalf("stale codex state persisted: %s", gotData)
	}
}

func TestAIWorkingLabelsPersistPerAgentUntilWorkingEnds(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	session := "dotfiles"

	if err := setAIState(session, "1_0", "codex", "CODEXING"); err != nil {
		t.Fatalf("setAIState working: %v", err)
	}
	state := loadSessionStateWithLegacy(session)
	codexLabel := state.AIWorkingLabels["codex"]
	if codexLabel == "" {
		t.Fatalf("working label missing: %#v", state)
	}

	if err := setAIState(session, "1_0", "codex", "CODEXING"); err != nil {
		t.Fatalf("setAIState working again: %v", err)
	}
	state = loadSessionStateWithLegacy(session)
	if state.AIWorkingLabels["codex"] != codexLabel {
		t.Fatalf("working label changed: %q -> %q", codexLabel, state.AIWorkingLabels["codex"])
	}

	if err := setAIState(session, "1_1", "claude", "CLAUDING"); err != nil {
		t.Fatalf("setAIState claude working: %v", err)
	}
	state = loadSessionStateWithLegacy(session)
	claudeLabel := state.AIWorkingLabels["claude"]
	if claudeLabel == "" {
		t.Fatalf("Claude working label missing: %#v", state)
	}
	if claudeLabel == codexLabel {
		t.Fatalf("Claude and Codex share label %q", claudeLabel)
	}

	if err := setAIState(session, "1_0", "codex", "WAITING"); err != nil {
		t.Fatalf("setAIState waiting: %v", err)
	}
	state = loadSessionStateWithLegacy(session)
	if state.AIWorkingLabels["codex"] != "" {
		t.Fatalf("Codex label kept after waiting: %#v", state.AIWorkingLabels)
	}
	if state.AIWorkingLabels["claude"] != claudeLabel {
		t.Fatalf("Claude label changed: %q -> %q", claudeLabel, state.AIWorkingLabels["claude"])
	}

	if err := setAIState(session, "1_1", "claude", "WAITING"); err != nil {
		t.Fatalf("setAIState claude waiting: %v", err)
	}
	state = loadSessionStateWithLegacy(session)
	if len(state.AIWorkingLabels) != 0 {
		t.Fatalf("working labels kept after waiting: %#v", state.AIWorkingLabels)
	}
}
