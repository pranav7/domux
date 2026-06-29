package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestResumeGroupStripsWorkspaceSuffix(t *testing.T) {
	got := resumeGroup("/home/me/proj/.domux/worktrees/workspace-2")
	if got != "proj" {
		t.Fatalf("resumeGroup = %q, want proj", got)
	}
	if got := resumeGroup("/home/me/proj"); got != "proj" {
		t.Fatalf("resumeGroup(main) = %q, want proj", got)
	}
}

func TestPlanResumePartitionsMissingRoots(t *testing.T) {
	live := t.TempDir()
	states := []SessionState{
		{Name: "alive", Root: live},
		{Name: "dead", Root: filepath.Join(t.TempDir(), "gone")},
	}

	recreate, prune := planResume(states, "")
	if len(recreate) != 1 || recreate[0].Name != "alive" {
		t.Fatalf("recreate = %#v, want [alive]", recreate)
	}
	if len(prune) != 1 || prune[0].Name != "dead" || !prune[0].Prune {
		t.Fatalf("prune = %#v, want [dead] with Prune=true", prune)
	}
}

func TestPlanResumeWorkspaceSharesMainGroup(t *testing.T) {
	base := t.TempDir()
	main := filepath.Join(base, "proj")
	ws := filepath.Join(main, ".domux", "worktrees", "workspace-1")
	if err := os.MkdirAll(ws, 0755); err != nil {
		t.Fatal(err)
	}
	states := []SessionState{
		{Name: "proj", Root: main},
		{Name: "workspace-1", Root: ws},
	}

	recreate, prune := planResume(states, "proj")
	if len(prune) != 0 {
		t.Fatalf("prune = %#v, want empty", prune)
	}
	if len(recreate) != 2 {
		t.Fatalf("recreate = %#v, want both states (shared group)", recreate)
	}
}

func TestPlanResumeFilterCaseInsensitive(t *testing.T) {
	base := t.TempDir()
	main := filepath.Join(base, "proj")
	if err := os.MkdirAll(main, 0755); err != nil {
		t.Fatal(err)
	}
	states := []SessionState{{Name: "proj", Root: main}}

	recreate, _ := planResume(states, "PROJ")
	if len(recreate) != 1 {
		t.Fatalf("recreate = %#v, want 1 (case-insensitive match)", recreate)
	}
}

func TestPlanResumeSkipsBlankNameOrRoot(t *testing.T) {
	states := []SessionState{
		{Name: "", Root: t.TempDir()},
		{Name: "noroot", Root: ""},
	}
	recreate, prune := planResume(states, "")
	if len(recreate) != 0 || len(prune) != 0 {
		t.Fatalf("recreate=%#v prune=%#v, want both empty", recreate, prune)
	}
}

func TestExecuteResumeStepRecreatesMissingSession(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session) exit 1 ;;
new-session) exit 0 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())
	root := t.TempDir()

	got := executeResumeStep(resumeTarget{Name: "sess", Root: root})
	if got.Err != nil {
		t.Fatalf("unexpected err: %v", got.Err)
	}
	if got.Status != resumeRecreated {
		t.Fatalf("Status = %q, want recreated", got.Status)
	}
	data, _ := os.ReadFile(callFile)
	if !strings.Contains(string(data), "new-session") {
		t.Fatalf("tmux new-session not invoked; calls=%q", data)
	}
	st, err := loadSessionState("sess")
	if err != nil || st.Root != root {
		t.Fatalf("session state Root = %q (err %v), want %q", st.Root, err, root)
	}
}

func TestExecuteResumeStepSkipsExistingSession(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session) exit 0 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	got := executeResumeStep(resumeTarget{Name: "sess", Root: t.TempDir()})
	if got.Status != resumeRunning {
		t.Fatalf("Status = %q, want already running", got.Status)
	}
	data, _ := os.ReadFile(callFile)
	if strings.Contains(string(data), "new-session") {
		t.Fatalf("should not create existing session; calls=%q", data)
	}
}

func TestExecuteResumeStepPruneRemovesState(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := saveSessionState(&SessionState{Name: "dead", Root: "/nonexistent"}); err != nil {
		t.Fatal(err)
	}
	legacy := filepath.Join(home, ".tmux-label-dead")
	if err := os.WriteFile(legacy, []byte("x"), 0644); err != nil {
		t.Fatal(err)
	}

	got := executeResumeStep(resumeTarget{Name: "dead", Prune: true})
	if got.Status != resumePruned || got.Err != nil {
		t.Fatalf("Status=%q Err=%v, want pruned/nil", got.Status, got.Err)
	}
	p, _ := sessionStatePath("dead")
	if fileExists(p) {
		t.Fatalf("state file not removed: %s", p)
	}
	if fileExists(legacy) {
		t.Fatalf("legacy file not removed: %s", legacy)
	}
}

func TestResumeJobRecordAndNext(t *testing.T) {
	j := &resumeJob{queue: []resumeTarget{{Name: "a"}, {Name: "b"}}}

	first, ok := j.nextTarget()
	if !ok || first.Name != "a" {
		t.Fatalf("nextTarget = %q ok=%v, want a", first.Name, ok)
	}
	j.record(resumeTarget{Name: "a", Status: resumeRecreated})
	if j.pos != 1 || j.nRecreated != 1 {
		t.Fatalf("pos=%d nRecreated=%d, want 1/1", j.pos, j.nRecreated)
	}
	second, ok := j.nextTarget()
	if !ok || second.Name != "b" {
		t.Fatalf("nextTarget = %q ok=%v, want b", second.Name, ok)
	}
	j.record(resumeTarget{Name: "b", Status: resumePruned})
	if _, ok := j.nextTarget(); ok {
		t.Fatalf("nextTarget should be exhausted")
	}
}

func TestResumeJobRecordCountsByStatus(t *testing.T) {
	j := &resumeJob{}
	j.record(resumeTarget{Status: resumeRecreated})
	j.record(resumeTarget{Status: resumeRunning})
	j.record(resumeTarget{Status: resumePruned})
	j.record(resumeTarget{Err: os.ErrPermission})
	if j.nRecreated != 1 || j.nRunning != 1 || j.nPruned != 1 || j.nFailed != 1 {
		t.Fatalf("counts = %#v, want 1 each", j)
	}
}

func TestResumeJobSummary(t *testing.T) {
	j := &resumeJob{nRecreated: 6, nPruned: 1}
	if got := j.summary(); got != "restored 6 · running 0 · pruned 1" {
		t.Fatalf("summary = %q", got)
	}
	j.nFailed = 2
	if got := j.summary(); !strings.Contains(got, "2 failed") {
		t.Fatalf("summary = %q, want failed count", got)
	}
}

func TestResumeStepCmdPrunes(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := saveSessionState(&SessionState{Name: "dead", Root: "/nonexistent"}); err != nil {
		t.Fatal(err)
	}

	msg := resumeStepCmd(resumeTarget{Name: "dead", Prune: true})()
	step, ok := msg.(resumeStepMsg)
	if !ok {
		t.Fatalf("msg type = %T, want resumeStepMsg", msg)
	}
	if step.target.Status != resumePruned {
		t.Fatalf("Status = %q, want pruned", step.target.Status)
	}
}
