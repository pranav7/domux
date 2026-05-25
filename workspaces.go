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

type workspaceResult struct {
	Path       string
	Branch     string
	Session    string
	BaseBranch string
	Slot       int
}

// provisionWorkspace creates the next available workspace-N worktree under
// <root>/.baag/worktrees, creates (or force-resets) the same-named branch
// from origin/<defaultBase>, and spins up a tmux session at the new path.
func provisionWorkspace(root string) (workspaceResult, error) {
	base, err := defaultBaseBranch(root)
	if err != nil {
		return workspaceResult{}, err
	}
	if out, err := exec.Command("git", "-C", root, "fetch", "-q", "origin", base).CombinedOutput(); err != nil {
		return workspaceResult{}, fmt.Errorf("git fetch origin %s: %w: %s", base, err, strings.TrimSpace(string(out)))
	}

	slot, err := lowestFreeWorkspaceSlot(root)
	if err != nil {
		return workspaceResult{}, err
	}
	branch := workspaceBranch(slot)
	path := workspacePath(root, slot)

	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return workspaceResult{}, fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}

	branchExists := gitBranchExists(root, branch)
	originRef := "origin/" + base

	if branchExists {
		if out, err := exec.Command("git", "-C", root, "branch", "-f", branch, originRef).CombinedOutput(); err != nil {
			return workspaceResult{}, fmt.Errorf("git branch -f %s %s: %w: %s", branch, originRef, err, strings.TrimSpace(string(out)))
		}
		if out, err := exec.Command("git", "-C", root, "worktree", "add", path, branch).CombinedOutput(); err != nil {
			return workspaceResult{}, fmt.Errorf("git worktree add: %w: %s", err, strings.TrimSpace(string(out)))
		}
	} else {
		if out, err := exec.Command("git", "-C", root, "worktree", "add", "-b", branch, path, originRef).CombinedOutput(); err != nil {
			return workspaceResult{}, fmt.Errorf("git worktree add -b: %w: %s", err, strings.TrimSpace(string(out)))
		}
	}

	session := uniqueTmuxSessionName(branch)
	if err := createTmuxSession(session, path); err != nil {
		return workspaceResult{}, err
	}
	if _, err := setSessionRoot(session, path); err != nil {
		return workspaceResult{}, err
	}

	return workspaceResult{
		Path:       path,
		Branch:     branch,
		Session:    session,
		BaseBranch: base,
		Slot:       slot,
	}, nil
}

func gitBranchExists(root, branch string) bool {
	cmd := exec.Command("git", "-C", root, "show-ref", "--verify", "--quiet", "refs/heads/"+branch)
	return cmd.Run() == nil
}

func uniqueTmuxSessionName(base string) string {
	if !tmuxSessionExists(base) {
		return base
	}
	for i := 2; ; i++ {
		candidate := fmt.Sprintf("%s-%d", base, i)
		if !tmuxSessionExists(candidate) {
			return candidate
		}
	}
}

// errDirtyWorkspace signals to the picker that it should re-prompt with the
// force confirmation. Anything else from removeWorkspace is a hard error.
var errDirtyWorkspace = fmt.Errorf("workspace has uncommitted or unpushed changes")

// removeWorkspace tears down a workspace-N worktree: tmux session(s),
// git worktree, branch, and any leftover legacy state files.
// Refuses if the path doesn't actually point at a workspace-N dir under
// <root>/.baag/worktrees/, even if the slot number is right.
func removeWorkspace(root string, slot int, force bool) error {
	if slot < 1 {
		return fmt.Errorf("invalid workspace slot %d", slot)
	}
	branch := workspaceBranch(slot)
	path := workspacePath(root, slot)

	if !isWorkspaceDir(filepath.Base(path)) {
		return fmt.Errorf("not a workspace dir: %s", path)
	}
	if !fileExists(path) {
		return fmt.Errorf("workspace path missing: %s", path)
	}

	if !force {
		if dirty, err := workspaceIsDirty(path, branch); err != nil {
			return err
		} else if dirty {
			return errDirtyWorkspace
		}
	}

	if err := killTmuxSessionsForRoot(path); err != nil {
		return err
	}

	args := []string{"-C", root, "worktree", "remove", path}
	if force {
		args = []string{"-C", root, "worktree", "remove", "--force", path}
	}
	if out, err := exec.Command("git", args...).CombinedOutput(); err != nil {
		return fmt.Errorf("git worktree remove: %w: %s", err, strings.TrimSpace(string(out)))
	}
	if out, err := exec.Command("git", "-C", root, "branch", "-D", branch).CombinedOutput(); err != nil {
		// Branch may not exist if provision crashed mid-way; tolerate that.
		if !strings.Contains(string(out), "not found") {
			return fmt.Errorf("git branch -D %s: %w: %s", branch, err, strings.TrimSpace(string(out)))
		}
	}
	return nil
}

func workspaceIsDirty(path, branch string) (bool, error) {
	statusOut, err := exec.Command("git", "-C", path, "status", "--porcelain").Output()
	if err != nil {
		return false, fmt.Errorf("git status: %w", err)
	}
	if len(strings.TrimSpace(string(statusOut))) > 0 {
		return true, nil
	}
	// Unpushed: prefer @{u}, else fall back to origin/<base>.
	upRange := branch + "@{u}.." + branch
	if err := exec.Command("git", "-C", path, "rev-parse", branch+"@{u}").Run(); err != nil {
		base, err := defaultBaseBranch(path)
		if err != nil {
			return false, err
		}
		upRange = "origin/" + base + ".." + branch
	}
	logOut, err := exec.Command("git", "-C", path, "log", "--oneline", upRange).Output()
	if err != nil {
		return false, fmt.Errorf("git log %s: %w", upRange, err)
	}
	return len(strings.TrimSpace(string(logOut))) > 0, nil
}

func killTmuxSessionsForRoot(path string) error {
	states, err := listSessionStates()
	if err != nil {
		return err
	}
	homeDir, _ := os.UserHomeDir()
	for _, st := range states {
		if st.Root != path {
			continue
		}
		_ = exec.Command("tmux", "kill-session", "-t", st.Name).Run()
		if homeDir != "" {
			_ = clearSessionStateFiles(homeDir, st.Name)
		}
		_ = removeSessionState(st.Name)
	}
	return nil
}
