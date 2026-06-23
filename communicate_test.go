package main

import (
	"strings"
	"testing"
)

func st(name, root, label string, ai map[string]string) SessionState {
	return SessionState{Name: name, Root: root, Label: label, AI: ai}
}

func TestMatchSessionsByNameMatchesNameRootBasenameAndLabel(t *testing.T) {
	states := []SessionState{
		st("workspace-2", "/repo/.domux/worktrees/workspace-2", "", nil),
		st("api", "/repo/api", "Billing fix", nil),
		st("docs", "/repo/docs", "", nil),
	}
	cases := []struct {
		name string
		want string // expected single match session name, or "" for none
		n    int    // expected match count
	}{
		{"workspace-2", "workspace-2", 1}, // by session name
		{"WORKSPACE-2", "workspace-2", 1}, // case-insensitive
		{"api", "api", 1},                 // by name
		{"Billing fix", "api", 1},         // by label
		{"docs", "docs", 1},               // by root basename + name
		{"nope", "", 0},
	}
	for _, c := range cases {
		got := matchSessionsByName(c.name, states)
		if len(got) != c.n {
			t.Fatalf("matchSessionsByName(%q) = %d matches, want %d", c.name, len(got), c.n)
		}
		if c.n == 1 && got[0].Name != c.want {
			t.Fatalf("matchSessionsByName(%q) = %q, want %q", c.name, got[0].Name, c.want)
		}
	}
}

func TestMatchSessionsByNameAmbiguousReturnsAll(t *testing.T) {
	states := []SessionState{
		st("alpha", "/repo/alpha", "", nil),
		st("beta", "/repo/beta", "alpha", nil), // label collides with alpha's name
	}
	got := matchSessionsByName("alpha", states)
	if len(got) != 2 {
		t.Fatalf("ambiguous name should match 2 sessions, got %d", len(got))
	}
}

func TestNormalizePaneSpec(t *testing.T) {
	cases := []struct {
		in   string
		want string
		ok   bool
	}{
		{"1_0", "1.0", true},
		{"1.0", "1.0", true},
		{"2_1", "2.1", true},
		{" 1.0 ", "1.0", true},
		{"default", "", false},
		{"", "", false},
		{"abc", "", false},
		{"1", "", false},
	}
	for _, c := range cases {
		got, ok := normalizePaneSpec(c.in)
		if got != c.want || ok != c.ok {
			t.Fatalf("normalizePaneSpec(%q) = (%q,%v), want (%q,%v)", c.in, got, ok, c.want, c.ok)
		}
	}
}

func TestClaudePaneSpecFromKey(t *testing.T) {
	cases := []struct {
		in   string
		want string
		ok   bool
	}{
		{"claude:1_0", "1.0", true},
		{"claude:2_1", "2.1", true},
		{"codex:1_0", "", false},
		{"claude:legacy", "", false},
		{"claude:default", "", false},
		{"claude:", "", false},
		{"garbage", "", false},
	}
	for _, c := range cases {
		got, ok := claudePaneSpecFromKey(c.in)
		if got != c.want || ok != c.ok {
			t.Fatalf("claudePaneSpecFromKey(%q) = (%q,%v), want (%q,%v)", c.in, got, ok, c.want, c.ok)
		}
	}
}

func TestClaudePaneSpecsFromStateSortedAndFiltered(t *testing.T) {
	s := st("ws", "/r", "", map[string]string{
		"claude:2_0":    "CLAUDING",
		"claude:1_0":    "WAITING",
		"codex:1_0":     "CODEXING",
		"claude:legacy": "CLAUDING",
	})
	got := claudePaneSpecsFromState(&s)
	if strings.Join(got, ",") != "1.0,2.0" {
		t.Fatalf("claudePaneSpecsFromState = %v, want [1.0 2.0]", got)
	}
	if claudePaneSpecsFromState(nil) != nil {
		t.Fatalf("nil state should yield nil specs")
	}
}

func TestLooksLikeClaudeCommand(t *testing.T) {
	cases := map[string]bool{
		"2.1.186":  true,
		"2.1":      true,
		"1.0.0":    true,
		"claude":   true,
		"Claude":   true,
		"2.1.nvim": false,
		"1.0x":     false,
		"zsh":      false,
		"bash":     false,
		"nvim":     false,
		"node":     false,
		"":         false,
	}
	for in, want := range cases {
		if got := looksLikeClaudeCommand(in); got != want {
			t.Fatalf("looksLikeClaudeCommand(%q) = %v, want %v", in, got, want)
		}
	}
}

func TestParsePaneLines(t *testing.T) {
	out := "workspace-2:1.0\t2.1.186\tw-2 task\t/repo/ws2\n" +
		"workspace-2:2.0\tzsh\tshell\t/repo/ws2\n\n"
	panes := parsePaneLines(out)
	if len(panes) != 2 {
		t.Fatalf("parsePaneLines = %d panes, want 2", len(panes))
	}
	if panes[0].Spec != "workspace-2:1.0" || panes[0].Command != "2.1.186" || panes[0].Title != "w-2 task" || panes[0].Path != "/repo/ws2" {
		t.Fatalf("first pane parsed wrong: %+v", panes[0])
	}
	if panes[1].Command != "zsh" {
		t.Fatalf("second pane command = %q, want zsh", panes[1].Command)
	}
}

func TestSplitTarget(t *testing.T) {
	s, p := splitTarget("workspace-2:1.0")
	if s != "workspace-2" || p != "1.0" {
		t.Fatalf("splitTarget = (%q,%q), want (workspace-2,1.0)", s, p)
	}
	s, p = splitTarget("noColon")
	if s != "noColon" || p != "" {
		t.Fatalf("splitTarget(noColon) = (%q,%q)", s, p)
	}
}

func TestResolveCommTargetFromStates(t *testing.T) {
	states := []SessionState{
		st("workspace-2", "/r/workspace-2", "", map[string]string{"claude:1_0": "WAITING"}),
		st("multi", "/r/multi", "", map[string]string{"claude:1_0": "CLAUDING", "claude:2_0": "WAITING"}),
		st("shellonly", "/r/shellonly", "", nil),
		st("dup", "/r/dup", "", nil),
		st("dup2", "/r/other", "dup", nil),
	}

	// 1. Single claude pane in AI map → resolved, no live needed.
	tgt, live, err := resolveCommTargetFromStates("workspace-2", "", states)
	if err != nil || live {
		t.Fatalf("workspace-2: err=%v live=%v", err, live)
	}
	if tgt.Target != "workspace-2:1.0" {
		t.Fatalf("workspace-2 target = %q, want workspace-2:1.0", tgt.Target)
	}

	// 2. Explicit --pane wins.
	tgt, live, err = resolveCommTargetFromStates("multi", "2.0", states)
	if err != nil || live || tgt.Target != "multi:2.0" {
		t.Fatalf("multi --pane 2.0 → target=%q live=%v err=%v", tgt.Target, live, err)
	}

	// 3. Multiple claude panes, no --pane → error.
	if _, _, err := resolveCommTargetFromStates("multi", "", states); err == nil {
		t.Fatalf("multi without --pane should be an error")
	}

	// 4. No claude pane in state → needsLive.
	_, live, err = resolveCommTargetFromStates("shellonly", "", states)
	if err != nil || !live {
		t.Fatalf("shellonly should need live lookup: live=%v err=%v", live, err)
	}

	// 5. No state match → needsLive with Session=name.
	tgt, live, err = resolveCommTargetFromStates("ghost", "", states)
	if err != nil || !live || tgt.Session != "ghost" {
		t.Fatalf("ghost → live=%v err=%v session=%q", live, err, tgt.Session)
	}

	// 6. Ambiguous name → error.
	if _, _, err := resolveCommTargetFromStates("dup", "", states); err == nil {
		t.Fatalf("ambiguous 'dup' should error")
	}

	// 7. Bad --pane → error.
	if _, _, err := resolveCommTargetFromStates("workspace-2", "garbage", states); err == nil {
		t.Fatalf("bad --pane should error")
	}
}
