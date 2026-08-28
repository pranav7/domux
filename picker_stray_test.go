package main

import (
	"os"
	"path/filepath"
	"testing"
)

// Real `tmux list-windows -a -F '#{session_name}\t#{?automatic-rename,1,0}'`
// output captured from a live server that had two agent leftovers
// (fixture-stack, pr1-stack) alongside real work.
const liveAutoNameOutput = "audrey\t0\n" +
	"audrey\t0\n" +
	"audrey-app\t0\n" +
	"audreyai_azure_tf_internal\t1\n" +
	"domux\t0\n" +
	"dotfiles\t0\n" +
	"fixture-stack\t1\n" +
	"notes\t0\n" +
	"notes\t0\n" +
	"pr1-stack\t1\n" +
	"workspace-1\t0\n" +
	"workspace-1\t0\n" +
	"workspace-2\t0\n" +
	"workspace-4\t0\n"

func TestIsStrayHidesUnmanagedIdleShellSession(t *testing.T) {
	probe := straySessionProbe{AllAutoNamed: true, PaneCmds: []string{"zsh"}}

	if !probe.isStray() {
		t.Fatalf("detached, unmanaged, auto-named session running only zsh should be stray")
	}
}

func TestIsStrayKeepsSessionDomuxTracks(t *testing.T) {
	// audreyai_azure_tf_internal looks exactly like a leftover — auto-named
	// window, idle zsh, detached — and is saved only by its state file.
	probe := straySessionProbe{HasDomuxState: true, AllAutoNamed: true, PaneCmds: []string{"zsh"}}

	if probe.isStray() {
		t.Fatalf("session with domux state should never be stray")
	}
}

func TestIsStrayKeepsAttachedSession(t *testing.T) {
	probe := straySessionProbe{Attached: true, AllAutoNamed: true, PaneCmds: []string{"zsh"}}

	if probe.isStray() {
		t.Fatalf("attached session should never be stray")
	}
}

func TestIsStrayKeepsNamedWindow(t *testing.T) {
	// A window you renamed by hand is deliberate even when it holds only a shell.
	probe := straySessionProbe{PaneCmds: []string{"zsh"}}

	if probe.isStray() {
		t.Fatalf("session with a manually named window should never be stray")
	}
}

func TestIsStrayKeepsSessionRunningSomething(t *testing.T) {
	probe := straySessionProbe{AllAutoNamed: true, PaneCmds: []string{"nvim"}}

	if probe.isStray() {
		t.Fatalf("session running nvim should never be stray")
	}
}

func TestIsStrayKeepsSessionWithOneBusyPane(t *testing.T) {
	probe := straySessionProbe{AllAutoNamed: true, PaneCmds: []string{"zsh", "node", "zsh"}}

	if probe.isStray() {
		t.Fatalf("session with one busy pane among idle shells should never be stray")
	}
}

func TestIsStrayKeepsSessionWithNoPaneData(t *testing.T) {
	// An unreadable pane probe must fail toward showing the session: hiding on
	// missing evidence would make sessions vanish whenever tmux hiccups.
	probe := straySessionProbe{AllAutoNamed: true}

	if probe.isStray() {
		t.Fatalf("session with no pane data should never be stray")
	}
}

func TestIsIdleShellCommand(t *testing.T) {
	idle := []string{"sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "csh", "tcsh", "login"}
	for _, cmd := range idle {
		if !isIdleShellCommand(cmd) {
			t.Errorf("isIdleShellCommand(%q) = false, want true", cmd)
		}
	}

	// "2.1.247" is how tmux reports a pane running the Claude CLI.
	busy := []string{"nvim", "node", "codex", "opencode", "2.1.247", "go", "ssh", "", "bashrc"}
	for _, cmd := range busy {
		if isIdleShellCommand(cmd) {
			t.Errorf("isIdleShellCommand(%q) = true, want false", cmd)
		}
	}
}

func TestParseAutoNamedSessions(t *testing.T) {
	got := parseAutoNamedSessions(liveAutoNameOutput)

	want := map[string]bool{
		"audrey":                     false,
		"audrey-app":                 false,
		"audreyai_azure_tf_internal": true,
		"domux":                      false,
		"dotfiles":                   false,
		"fixture-stack":              true,
		"notes":                      false,
		"pr1-stack":                  true,
		"workspace-1":                false,
		"workspace-2":                false,
		"workspace-4":                false,
	}
	if len(got) != len(want) {
		t.Fatalf("parsed %d sessions, want %d: %#v", len(got), len(want), got)
	}
	for session, wantAuto := range want {
		if got[session] != wantAuto {
			t.Errorf("%s auto-named = %v, want %v", session, got[session], wantAuto)
		}
	}
}

func TestParseAutoNamedSessionsNeedsEveryWindowAutoNamed(t *testing.T) {
	got := parseAutoNamedSessions("mixed\t1\nmixed\t0\nmixed\t1\n")

	if got["mixed"] {
		t.Fatalf("session with one manually named window should not count as auto-named")
	}
}

func TestParseAutoNamedSessionsOnEmptyOutput(t *testing.T) {
	if got := parseAutoNamedSessions("\n"); len(got) != 0 {
		t.Fatalf("parseAutoNamedSessions on empty output = %#v, want empty", got)
	}
}

func TestParsePaneCommandsBySession(t *testing.T) {
	got := parsePaneCommandsBySession("kept\tzsh\nkept\tnvim\nstray\tbash\nmalformed\n")

	if want := []string{"zsh", "nvim"}; !equalStrings(got["kept"], want) {
		t.Errorf("kept panes = %#v, want %#v", got["kept"], want)
	}
	if want := []string{"bash"}; !equalStrings(got["stray"], want) {
		t.Errorf("stray panes = %#v, want %#v", got["stray"], want)
	}
	if _, ok := got["malformed"]; ok {
		t.Errorf("line without a command should be skipped, got %#v", got)
	}
}

func TestParseSessionListLines(t *testing.T) {
	got := parseSessionListLines("attached\t1\ndetached\t0\n\nnoflag\n")

	want := []liveSession{
		{Name: "attached", Attached: true},
		{Name: "detached", Attached: false},
		{Name: "noflag", Attached: false},
	}
	if len(got) != len(want) {
		t.Fatalf("parsed %#v, want %#v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("session %d = %#v, want %#v", i, got[i], want[i])
		}
	}
}

func TestGatherSessionsOmitsStraySession(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)

	sessionsDir := filepath.Join(homeDir, ".local", "share", "domux", "sessions")
	if err := os.MkdirAll(sessionsDir, 0755); err != nil {
		t.Fatalf("mkdir sessions: %v", err)
	}
	if err := os.WriteFile(filepath.Join(sessionsDir, "kept.json"), []byte(`{"name":"kept"}`), 0600); err != nil {
		t.Fatalf("write kept state: %v", err)
	}

	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
case "$1 $2" in
"list-sessions -F") printf 'kept\t0\nstray\t0\n' ;;
"list-windows -a") printf 'kept\t1\nstray\t1\n' ;;
"list-panes -a") printf 'kept\tzsh\nstray\tzsh\n' ;;
"display-message -t") echo "/nonexistent" ;;
"list-windows -t") printf '1\tzsh\t1\t/nonexistent\n' ;;
"list-panes -s") printf '1\t/dev/ttys001\n' ;;
esac
exit 0
`, callFile)

	var names []string
	for _, row := range gatherSessions() {
		if row.Kind == rowSession && row.Session != nil {
			names = append(names, row.Session.Name)
		}
	}

	if want := []string{"kept"}; !equalStrings(names, want) {
		t.Fatalf("gatherSessions sessions = %#v, want %#v", names, want)
	}
}

func equalStrings(got, want []string) bool {
	if len(got) != len(want) {
		return false
	}
	for i := range got {
		if got[i] != want[i] {
			return false
		}
	}
	return true
}
