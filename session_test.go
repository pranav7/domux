package main

import (
	"path/filepath"
	"testing"
)

func TestAggregateAIStatesByWindow(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"claude:1_0":   "CLAUDING",
		"claude:2_0":   "WAITING",
		"codex:2_0":    "CODEXING",
		"opencode:2_2": "CODING",
		"claude:2_1":   "CLAUDING", // second pane in window 2; WAITING must still win
	}}

	got := aggregateAIStatesByWindow(state)

	if got[1].Claude != "CLAUDING" {
		t.Errorf("window 1 Claude = %q, want CLAUDING", got[1].Claude)
	}
	if got[2].Claude != "WAITING" {
		t.Errorf("window 2 Claude = %q, want WAITING (WAITING outranks CLAUDING)", got[2].Claude)
	}
	if got[2].Codex != "CODEXING" {
		t.Errorf("window 2 Codex = %q, want CODEXING", got[2].Codex)
	}
	if got[2].OpenCode != "CODING" {
		t.Errorf("window 2 OpenCode = %q, want CODING", got[2].OpenCode)
	}
	if _, ok := got[3]; ok {
		t.Errorf("window 3 should be absent, got %+v", got[3])
	}
}

func TestAggregateAIStatesByWindowNilAndEmpty(t *testing.T) {
	if got := aggregateAIStatesByWindow(nil); len(got) != 0 {
		t.Errorf("nil state = %+v, want empty map", got)
	}
	if got := aggregateAIStatesByWindow(&SessionState{}); len(got) != 0 {
		t.Errorf("empty state = %+v, want empty map", got)
	}
}

func TestSnapshotWindowsParsesTmuxOutput(t *testing.T) {
	installFakeTmux(t, `#!/bin/sh
printf '1\tmain\t/p/domux\n2\tmerge queue\t/p/audrey\n3\tscratch\t/p/x\n'
`, filepath.Join(t.TempDir(), "call"))

	got := snapshotWindows("sess")
	want := []WindowSnapshot{
		{Index: 1, Name: "main", Cwd: "/p/domux"},
		{Index: 2, Name: "merge queue", Cwd: "/p/audrey"},
		{Index: 3, Name: "scratch", Cwd: "/p/x"},
	}
	if len(got) != len(want) {
		t.Fatalf("snapshotWindows = %+v, want %+v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("window %d = %+v, want %+v", i, got[i], want[i])
		}
	}
}

func TestSnapshotWindowsReturnsNilOnTmuxError(t *testing.T) {
	installFakeTmux(t, `#!/bin/sh
exit 1
`, filepath.Join(t.TempDir(), "call"))
	if got := snapshotWindows("sess"); got != nil {
		t.Fatalf("snapshotWindows on tmux error = %+v, want nil", got)
	}
}

func TestSaveSessionStateSnapshotsWindows(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	installFakeTmux(t, `#!/bin/sh
printf '1\tmain\t/p/domux\n2\tside\t/p/audrey\n'
`, filepath.Join(t.TempDir(), "call"))

	if err := saveSessionState(&SessionState{Name: "sess", Root: "/p/domux"}); err != nil {
		t.Fatal(err)
	}
	st, err := loadSessionState("sess")
	if err != nil {
		t.Fatal(err)
	}
	if len(st.Windows) != 2 || st.Windows[0].Name != "main" || st.Windows[1].Cwd != "/p/audrey" {
		t.Fatalf("state.Windows = %+v, want the 2 snapshotted windows", st.Windows)
	}
}

func TestSaveSessionStatePreservesWindowsWhenTmuxUnavailable(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// tmux errors → snapshotWindows returns nil → previously-saved Windows must
	// survive, not be wiped.
	installFakeTmux(t, `#!/bin/sh
exit 1
`, filepath.Join(t.TempDir(), "call"))

	seed := &SessionState{
		Name:    "sess",
		Root:    "/p/domux",
		Windows: []WindowSnapshot{{Index: 1, Name: "main", Cwd: "/p/domux"}},
	}
	if err := saveSessionState(seed); err != nil {
		t.Fatal(err)
	}
	st, err := loadSessionState("sess")
	if err != nil {
		t.Fatal(err)
	}
	if len(st.Windows) != 1 || st.Windows[0].Name != "main" {
		t.Fatalf("state.Windows = %+v, want the seeded window preserved", st.Windows)
	}
}
