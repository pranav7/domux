package main

import (
	"os"
	"os/exec"
	"strings"
	"testing"
)

// setupGitWorkspaceRepo creates root + bare origin, pushes an initial commit
// on `main`, and sets origin/HEAD → origin/main. Returns the working root.
func setupGitWorkspaceRepo(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	bare := t.TempDir()

	gitRun(t, bare, "init", "--bare", "-q", "-b", "main")
	gitRun(t, root, "init", "-q", "-b", "main")
	gitRun(t, root, "config", "user.email", "test@example.com")
	gitRun(t, root, "config", "user.name", "test")
	gitRun(t, root, "commit", "--allow-empty", "-q", "-m", "init")
	gitRun(t, root, "remote", "add", "origin", bare)
	gitRun(t, root, "push", "-q", "-u", "origin", "main")
	gitRun(t, root, "remote", "set-head", "origin", "main")
	return root
}

func gitRun(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %s in %s: %v\n%s", strings.Join(args, " "), dir, err, out)
	}
}

func gitOutput(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s in %s: %v\n%s", strings.Join(args, " "), dir, err, out)
	}
	return strings.TrimSpace(string(out))
}

func TestDefaultBaseBranchFromSymbolicRef(t *testing.T) {
	root := setupGitWorkspaceRepo(t)

	got, err := defaultBaseBranch(root)
	if err != nil {
		t.Fatalf("defaultBaseBranch: %v", err)
	}
	if got != "main" {
		t.Fatalf("got %q, want main", got)
	}
}

func TestDefaultBaseBranchFallsBackToMain(t *testing.T) {
	root := t.TempDir()
	gitRun(t, root, "init", "-q", "-b", "develop")
	gitRun(t, root, "config", "user.email", "t@t.t")
	gitRun(t, root, "config", "user.name", "t")
	gitRun(t, root, "commit", "--allow-empty", "-q", "-m", "init")
	// no origin → no symbolic-ref

	got, err := defaultBaseBranch(root)
	if err != nil {
		t.Fatalf("defaultBaseBranch: %v", err)
	}
	if got != "main" {
		t.Fatalf("got %q, want main fallback", got)
	}
}

func TestLowestFreeWorkspaceSlot(t *testing.T) {
	cases := []struct {
		name string
		dirs []int
		want int
	}{
		{"empty", nil, 1},
		{"contiguous", []int{1, 2}, 3},
		{"gap at start", []int{2, 3}, 1},
		{"gap middle", []int{1, 3}, 2},
		{"unsorted", []int{4, 1, 2}, 3},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			for _, n := range tc.dirs {
				if err := os.MkdirAll(workspacePath(root, n), 0755); err != nil {
					t.Fatalf("mkdir: %v", err)
				}
			}
			got, err := lowestFreeWorkspaceSlot(root)
			if err != nil {
				t.Fatalf("lowestFreeWorkspaceSlot: %v", err)
			}
			if got != tc.want {
				t.Fatalf("got %d, want %d", got, tc.want)
			}
		})
	}
}
