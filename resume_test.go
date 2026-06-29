package main

import (
	"os"
	"path/filepath"
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
