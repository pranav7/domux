package main

import (
	"os/exec"
	"reflect"
	"strings"
	"testing"
)

// mustRepoRoot returns this checkout's git root, skipping when git is absent.
func mustRepoRoot(t *testing.T) string {
	t.Helper()
	out, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		t.Skipf("git unavailable: %v", err)
	}
	return strings.TrimSpace(string(out))
}

// One `tmux list-panes -a` line per pane, in the field order of
// tmuxServerPaneFormat. Mirrors real output: an empty window name, a session
// with two windows, a window with two panes, attached and detached sessions.
const liveServerPaneOutput = "audrey\t0\t1\tgt au\t0\t0\t1\t2.1.250\t/dev/ttys002\t/Users/pranav/projects/audrey\n" +
	"audrey\t0\t2\tcodereview\t1\t0\t1\tnvim\t/dev/ttys003\t/Users/pranav/projects/audrey\n" +
	"domux\t1\t1\t\t1\t0\t1\t2.1.250\t/dev/ttys014\t/Users/pranav/projects/domux\n" +
	"dotfiles\t0\t1\tnvim\t1\t1\t0\tnvim\t/dev/ttys015\t/Users/pranav/dotfiles\n" +
	"dotfiles\t0\t1\tnvim\t1\t1\t1\tzsh\t/dev/ttys005\t/Users/pranav/dotfiles/sub\n"

func TestParseTmuxServerPanes(t *testing.T) {
	got := parseTmuxServerPanes(liveServerPaneOutput)
	if len(got) != 5 {
		t.Fatalf("parseTmuxServerPanes returned %d panes, want 5: %#v", len(got), got)
	}
	want := serverPane{
		Session: "audrey", Attached: false,
		WindowIndex: 1, WindowName: "gt au", WindowActive: false, AutoNamed: false,
		PaneActive: true, Command: "2.1.250", TTY: "/dev/ttys002",
		Path: "/Users/pranav/projects/audrey",
	}
	if got[0] != want {
		t.Errorf("pane[0] = %#v, want %#v", got[0], want)
	}
	if got[2].Session != "domux" || !got[2].Attached || got[2].WindowName != "" {
		t.Errorf("pane[2] = %#v, want attached domux pane with empty window name", got[2])
	}
	if !got[3].AutoNamed || got[3].PaneActive {
		t.Errorf("pane[3] = %#v, want auto-named non-active pane", got[3])
	}
}

func TestParseTmuxServerPanesSkipsMalformed(t *testing.T) {
	if got := parseTmuxServerPanes(""); len(got) != 0 {
		t.Errorf("parseTmuxServerPanes(\"\") = %#v, want empty", got)
	}
	// too few fields, and a non-integer window index
	bad := "only\tthree\tfields\n" + "sess\t0\tNaN\tw\t1\t0\t1\tzsh\t/dev/ttys1\t/tmp\n"
	if got := parseTmuxServerPanes(bad); len(got) != 0 {
		t.Errorf("parseTmuxServerPanes(malformed) = %#v, want empty", got)
	}
}

func TestTmuxServerSnapshotSessionPath(t *testing.T) {
	snap := newTmuxServerSnapshot(parseTmuxServerPanes(liveServerPaneOutput))

	// The session's current path is the active pane of its active window —
	// what `tmux display-message -t S -p '#{pane_current_path}'` returns.
	if got := snap.sessionPath("audrey"); got != "/Users/pranav/projects/audrey" {
		t.Errorf("sessionPath(audrey) = %q", got)
	}
	if got := snap.sessionPath("dotfiles"); got != "/Users/pranav/dotfiles/sub" {
		t.Errorf("sessionPath(dotfiles) = %q, want the active pane's path", got)
	}
	if got := snap.sessionPath("missing"); got != "" {
		t.Errorf("sessionPath(missing) = %q, want empty", got)
	}
}

func TestTmuxServerSnapshotWindows(t *testing.T) {
	snap := newTmuxServerSnapshot(parseTmuxServerPanes(liveServerPaneOutput))

	got := snap.windows("audrey")
	want := []windowInfo{
		{Index: 1, Name: "gt au", Active: false, Path: "/Users/pranav/projects/audrey"},
		{Index: 2, Name: "codereview", Active: true, Path: "/Users/pranav/projects/audrey"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("windows(audrey) = %#v, want %#v", got, want)
	}

	// A window with several panes collapses to one row carrying the active
	// pane's cwd (windowInfo.Path is documented as the active-pane cwd).
	got = snap.windows("dotfiles")
	if len(got) != 1 {
		t.Fatalf("windows(dotfiles) = %#v, want 1 row", got)
	}
	if got[0].Path != "/Users/pranav/dotfiles/sub" {
		t.Errorf("windows(dotfiles)[0].Path = %q, want the active pane's cwd", got[0].Path)
	}
	if got := snap.windows("missing"); got != nil {
		t.Errorf("windows(missing) = %#v, want nil", got)
	}
}

func TestTmuxServerSnapshotPaneTTYs(t *testing.T) {
	snap := newTmuxServerSnapshot(parseTmuxServerPanes(liveServerPaneOutput))

	// All of a window's pane ttys bucket together under its index.
	got := snap.paneTTYs("dotfiles")
	want := map[int][]string{1: {"/dev/ttys015", "/dev/ttys005"}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("paneTTYs(dotfiles) = %#v, want %#v", got, want)
	}
	if got := snap.paneTTYs("audrey"); len(got) != 2 {
		t.Errorf("paneTTYs(audrey) = %#v, want 2 windows", got)
	}
}

func TestTmuxServerSnapshotStrayInputs(t *testing.T) {
	snap := newTmuxServerSnapshot(parseTmuxServerPanes(liveServerPaneOutput))

	// dotfiles has every window on automatic-rename; audrey has none.
	if !snap.allAutoNamed("dotfiles") {
		t.Error("allAutoNamed(dotfiles) = false, want true")
	}
	if snap.allAutoNamed("audrey") {
		t.Error("allAutoNamed(audrey) = true, want false")
	}
	if got := snap.paneCommands("dotfiles"); !reflect.DeepEqual(got, []string{"nvim", "zsh"}) {
		t.Errorf("paneCommands(dotfiles) = %#v, want [nvim zsh]", got)
	}

	// One manually named window turns the whole session off auto-named.
	mixed := newTmuxServerSnapshot(parseTmuxServerPanes(
		"mixed\t0\t1\tauto\t1\t1\t1\tzsh\t/dev/ttys1\t/tmp\n" +
			"mixed\t0\t2\tnamed\t0\t0\t1\tzsh\t/dev/ttys2\t/tmp\n"))
	if mixed.allAutoNamed("mixed") {
		t.Error("session with one manually named window should not count as auto-named")
	}

	// An unreadable probe must fail toward showing the session, never hiding it.
	empty := newTmuxServerSnapshot(nil)
	if empty.allAutoNamed("anything") {
		t.Error("allAutoNamed on an empty snapshot = true, want false")
	}
	if got := empty.paneCommands("anything"); got != nil {
		t.Errorf("paneCommands on an empty snapshot = %#v, want nil", got)
	}
}

func TestParseGitRootBranch(t *testing.T) {
	tests := []struct {
		name string
		out  string
		want gitFacts
	}{
		{
			name: "root and branch",
			out:  "/Users/pranav/projects/domux\nmain\n",
			want: gitFacts{Root: "/Users/pranav/projects/domux", Branch: "main"},
		},
		{
			// Detached HEAD: --abbrev-ref echoes "HEAD", which is not a branch
			// name. `git branch --show-current` returned "" here, so must we.
			name: "detached head has no branch",
			out:  "/Users/pranav/projects/domux\nHEAD\n",
			want: gitFacts{Root: "/Users/pranav/projects/domux"},
		},
		{
			// An unborn HEAD (fresh `git init`) exits non-zero but still writes
			// the toplevel to stdout — keep the root we did get.
			name: "root only",
			out:  "/private/tmp/emptyrepo\n",
			want: gitFacts{Root: "/private/tmp/emptyrepo"},
		},
		{
			name: "not a repo",
			out:  "",
			want: gitFacts{},
		},
		{
			// Never mistake a non-path first line for a root.
			name: "relative first line is not a root",
			out:  "not-a-path\nmain\n",
			want: gitFacts{},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := parseGitRootBranch(tt.out); got != tt.want {
				t.Errorf("parseGitRootBranch(%q) = %#v, want %#v", tt.out, got, tt.want)
			}
		})
	}
}

func TestGitFactsForPathsDedupesAndSkipsEmpty(t *testing.T) {
	// Two sessions in the same worktree must cost one git call, and "" is never
	// probed. Against the live repo the root resolves to this checkout.
	repo := mustRepoRoot(t)
	got := gitFactsForPaths([]string{repo, repo, ""})
	if len(got) != 1 {
		t.Fatalf("gitFactsForPaths returned %d entries, want 1: %#v", len(got), got)
	}
	if got[repo].Root != repo {
		t.Errorf("gitFactsForPaths[%q].Root = %q, want %q", repo, got[repo].Root, repo)
	}
}
