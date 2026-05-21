package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
)

// defaultBaseBranch returns the short branch name that `origin/HEAD` points
// at (e.g. "main", "master", "develop"). Falls back to "main" if origin/HEAD
// isn't set — keeps backwards compat with the prior hardcoded behaviour.
func defaultBaseBranch(root string) (string, error) {
	cmd := exec.Command("git", "-C", root, "symbolic-ref", "--short", "refs/remotes/origin/HEAD")
	out, err := cmd.Output()
	if err != nil {
		return "main", nil
	}
	ref := strings.TrimSpace(string(out))
	ref = strings.TrimPrefix(ref, "origin/")
	if ref == "" {
		return "main", nil
	}
	return ref, nil
}

// workspaceWorktreeDir is the conventional location under <root> for
// numbered scratch worktrees. Matches the `.baag/worktrees/workspace-N`
// layout the picker already understands.
const workspaceWorktreeDir = ".baag/worktrees"

// lowestFreeWorkspaceSlot returns the lowest N >= 1 where no directory
// named workspace-N exists under <root>/.baag/worktrees. Existing git
// worktrees registered elsewhere with the same name aren't checked here —
// `git worktree add` will fail loudly if there's a conflict.
func lowestFreeWorkspaceSlot(root string) (int, error) {
	dir := filepath.Join(root, workspaceWorktreeDir)
	entries, err := os.ReadDir(dir)
	if err != nil && !os.IsNotExist(err) {
		return 0, fmt.Errorf("read %s: %w", dir, err)
	}
	taken := map[int]bool{}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		const prefix = "workspace-"
		if !strings.HasPrefix(e.Name(), prefix) {
			continue
		}
		suffix := strings.TrimPrefix(e.Name(), prefix)
		n, err := strconv.Atoi(suffix)
		if err != nil || n < 1 {
			continue
		}
		taken[n] = true
	}
	for n := 1; ; n++ {
		if !taken[n] {
			return n, nil
		}
	}
}

func workspacePath(root string, n int) string {
	return filepath.Join(root, workspaceWorktreeDir, fmt.Sprintf("workspace-%d", n))
}

func workspaceBranch(n int) string {
	return fmt.Sprintf("workspace-%d", n)
}
