package main

import (
	"os"
	"path/filepath"
	"testing"
)

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

// The stray filter reads its two whole-server facts off the shared pane
// snapshot; allAutoNamed and paneCommands are covered in tmux_snapshot_test.go.

func TestStrayFilterFromSnapshot(t *testing.T) {
	// fixture-stack is a leftover: no state file, detached, auto-named, idle zsh.
	// notes is saved by its manually named window alone.
	snap := newTmuxServerSnapshot(parseTmuxServerPanes(
		"fixture-stack\t0\t1\tzsh\t1\t1\t1\tzsh\t/dev/ttys020\t/tmp\n" +
			"notes\t0\t1\tnotes\t1\t0\t1\tzsh\t/dev/ttys021\t/tmp\n"))
	f := newStraySessionFilter(snap)

	if !f.isStray(liveSession{Name: "fixture-stack"}) {
		t.Error("fixture-stack should be stray")
	}
	if f.isStray(liveSession{Name: "notes"}) {
		t.Error("notes has a manually named window and should be kept")
	}
	// A session tmux told us nothing about must never vanish.
	if f.isStray(liveSession{Name: "unknown"}) {
		t.Error("session absent from the snapshot should be kept")
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
	// Both sessions look identical to tmux — detached, auto-named, idle zsh — so
	// only the state file written above separates kept from stray. The pane rows
	// are in tmuxServerPaneFormat order.
	installFakeTmux(t, `#!/bin/sh
case "$1 $2" in
"list-sessions -F") printf 'kept\t0\nstray\t0\n' ;;
"list-panes -a") printf 'kept\t0\t1\tzsh\t1\t1\t1\tzsh\t/dev/ttys001\t/nonexistent\nstray\t0\t1\tzsh\t1\t1\t1\tzsh\t/dev/ttys002\t/nonexistent\n' ;;
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
