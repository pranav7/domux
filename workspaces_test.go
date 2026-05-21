package main

import (
	"os"
	"os/exec"
	"path/filepath"
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

func TestProvisionWorkspaceCreatesWorktreeAndBranch(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session) exit 1 ;;
new-session) exit 0 ;;
attach-session|switch-client) exit 0 ;;
display-message) echo workspace-1 ; exit 0 ;;
list-sessions) echo workspace-1 ; exit 0 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	res, err := provisionWorkspace(root)
	if err != nil {
		t.Fatalf("provisionWorkspace: %v", err)
	}
	if res.Branch != "workspace-1" {
		t.Fatalf("Branch = %q, want workspace-1", res.Branch)
	}
	if !fileExists(res.Path) {
		t.Fatalf("worktree dir not created: %s", res.Path)
	}

	wantBranch := gitOutput(t, res.Path, "branch", "--show-current")
	if wantBranch != "workspace-1" {
		t.Fatalf("worktree branch = %q, want workspace-1", wantBranch)
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

func TestProvisionWorkspaceResetsExistingBranchToBase(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	// Pre-create workspace-1 branch pointing at a NEW commit ahead of main.
	gitRun(t, root, "checkout", "-q", "-b", "workspace-1")
	gitRun(t, root, "commit", "--allow-empty", "-q", "-m", "ahead")
	aheadSha := gitOutput(t, root, "rev-parse", "workspace-1")
	gitRun(t, root, "checkout", "-q", "main")

	mainSha := gitOutput(t, root, "rev-parse", "origin/main")
	if aheadSha == mainSha {
		t.Fatalf("setup did not advance workspace-1 ahead of origin/main")
	}

	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
case "$1" in
has-session) exit 1 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	res, err := provisionWorkspace(root)
	if err != nil {
		t.Fatalf("provisionWorkspace: %v", err)
	}

	gotSha := gitOutput(t, res.Path, "rev-parse", "HEAD")
	if gotSha != mainSha {
		t.Fatalf("worktree HEAD = %s, want origin/main %s (existing branch wasn't reset)", gotSha, mainSha)
	}
}

func TestRemoveWorkspaceCleansWorktreeAndBranch(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
case "$1" in
has-session) exit 1 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	res, err := provisionWorkspace(root)
	if err != nil {
		t.Fatalf("provisionWorkspace: %v", err)
	}

	if err := removeWorkspace(root, res.Slot, false); err != nil {
		t.Fatalf("removeWorkspace: %v", err)
	}
	if fileExists(res.Path) {
		t.Fatalf("worktree dir still exists: %s", res.Path)
	}
	if gitBranchExists(root, res.Branch) {
		t.Fatalf("branch %s still exists", res.Branch)
	}
}

func TestRemoveWorkspaceRefusesNonWorkspaceDir(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	// No workspace exists at slot 1.
	err := removeWorkspace(root, 1, false)
	if err == nil {
		t.Fatalf("expected error for missing workspace")
	}
}

func TestRemoveWorkspaceRefusesUncommittedWithoutForce(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
case "$1" in
has-session) exit 1 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	res, err := provisionWorkspace(root)
	if err != nil {
		t.Fatalf("provisionWorkspace: %v", err)
	}
	// Dirty the worktree.
	if err := os.WriteFile(filepath.Join(res.Path, "scratch.txt"), []byte("x"), 0644); err != nil {
		t.Fatalf("write scratch: %v", err)
	}

	err = removeWorkspace(root, res.Slot, false)
	if err == nil {
		t.Fatalf("expected errDirtyWorkspace, got nil")
	}
	if err != errDirtyWorkspace {
		t.Fatalf("err = %v, want errDirtyWorkspace", err)
	}
	if !fileExists(res.Path) {
		t.Fatalf("worktree removed despite dirty refuse")
	}
}

func TestRemoveWorkspaceForceRemovesDirty(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
case "$1" in
has-session) exit 1 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	res, err := provisionWorkspace(root)
	if err != nil {
		t.Fatalf("provisionWorkspace: %v", err)
	}
	if err := os.WriteFile(filepath.Join(res.Path, "scratch.txt"), []byte("x"), 0644); err != nil {
		t.Fatalf("write scratch: %v", err)
	}

	if err := removeWorkspace(root, res.Slot, true); err != nil {
		t.Fatalf("removeWorkspace force: %v", err)
	}
	if fileExists(res.Path) {
		t.Fatalf("worktree dir still exists after force remove")
	}
}
