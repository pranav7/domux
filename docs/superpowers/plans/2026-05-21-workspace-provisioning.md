# Workspace provisioning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `+` (create) and `D` (remove) workspace-N worktree management to the domux switcher TUI, plus a subtle marker for each group's main worktree.

**Architecture:** One new file `workspaces.go` shells out to `git` and `tmux` (matching domux's existing pattern). Picker (`picker.go`) grows two keybinds, a confirmDelete mode parallel to the existing `labelEditing` mode, and a glyph in `renderSession`. `resetGitWorkspace` switches from a hardcoded `"main"` to a `defaultBaseBranch` helper so add/remove/reset all agree on the base.

**Tech Stack:** Go 1.22, single `package main`, `bubbletea` TUI, no new deps.

**Spec:** `docs/superpowers/specs/2026-05-21-workspace-provisioning-design.md`

---

## File layout

- **Create** `workspaces.go` — `provisionWorkspace`, `removeWorkspace`, `defaultBaseBranch`, `lowestFreeWorkspaceSlot`, `isMainWorktreePath`, `errDirtyWorkspace`.
- **Create** `workspaces_test.go` — unit + integration tests with a real git temp repo and a tmux PATH shim.
- **Modify** `picker.go` — add `Root` field on `sessionInfo`, populate in `gatherSessions`, `+` and `D` keybinds, `confirmDelete` mode (incl. `force` re-prompt), `◇` glyph in `renderSession`, footer text updates.
- **Modify** `picker_test.go` — picker behavior tests for the new affordances.
- **Modify** `commands.go` — `resetGitWorkspace` calls `defaultBaseBranch(dir)` instead of literal `"main"`.

---

## Task 1: Test helpers for a real git repo

**Files:**
- Create: `workspaces_test.go`

We need a hermetic git repo with an `origin` remote and `origin/HEAD` set. Used by every subsequent integration-style test. Lives in `workspaces_test.go` so it's near its callers.

- [ ] **Step 1: Create `workspaces_test.go` with helpers**

```go
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

// fileExists is a tiny helper used in remove tests.
func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

// joinWorkspacePath returns root/.baag/worktrees/workspace-<n>.
func joinWorkspacePath(root string, n int) string {
	return filepath.Join(root, ".baag", "worktrees", "workspaceN")
}
```

The final helper has a deliberate sentinel name; we'll replace it after we have the real implementation (Task 3) — for now it just keeps the file compiling.

- [ ] **Step 2: Verify the file compiles and runs (no tests yet)**

Run: `go test ./... -run TestNoSuchTest`
Expected: `ok` with `[no tests to run]`.

- [ ] **Step 3: Commit**

```bash
git add workspaces_test.go
git commit -m "test: add git/tmux helpers for workspace provisioning tests"
```

---

## Task 2: `defaultBaseBranch` helper

**Files:**
- Create: `workspaces.go`
- Modify: `workspaces_test.go`

- [ ] **Step 1: Add the failing tests**

Append to `workspaces_test.go`:

```go
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test ./... -run TestDefaultBaseBranch -v`
Expected: FAIL — `undefined: defaultBaseBranch`.

- [ ] **Step 3: Create `workspaces.go` with the helper**

```go
package main

import (
	"os/exec"
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test ./... -run TestDefaultBaseBranch -v`
Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add workspaces.go workspaces_test.go
git commit -m "feat: default-base-branch helper for workspace provisioning"
```

---

## Task 3: `lowestFreeWorkspaceSlot` helper

**Files:**
- Modify: `workspaces.go`
- Modify: `workspaces_test.go`

- [ ] **Step 1: Add the failing test**

Append to `workspaces_test.go`:

```go
func TestLowestFreeWorkspaceSlot(t *testing.T) {
	cases := []struct {
		name  string
		dirs  []int
		want  int
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
			wt := filepath.Join(root, ".baag", "worktrees")
			for _, n := range tc.dirs {
				if err := os.MkdirAll(filepath.Join(wt, fmtWorkspaceName(n)), 0755); err != nil {
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

func fmtWorkspaceName(n int) string {
	return "workspace-" + itoa(n)
}

func itoa(n int) string {
	return strings.TrimPrefix(strings.TrimSpace(fmtInt(n)), "+")
}

func fmtInt(n int) string {
	// crude itoa to avoid pulling strconv into the helper namespace
	if n == 0 {
		return "0"
	}
	neg := n < 0
	if neg {
		n = -n
	}
	var buf [20]byte
	i := len(buf)
	for n > 0 {
		i--
		buf[i] = byte('0' + n%10)
		n /= 10
	}
	if neg {
		i--
		buf[i] = '-'
	}
	return string(buf[i:])
}
```

(The itoa nonsense is just to avoid adding `strconv` to the test imports we already touched; keep it.)

- [ ] **Step 2: Run test, verify it fails**

Run: `go test ./... -run TestLowestFreeWorkspaceSlot -v`
Expected: FAIL — `undefined: lowestFreeWorkspaceSlot`.

- [ ] **Step 3: Implement in `workspaces.go`**

Append to `workspaces.go`:

```go
import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
)

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
```

Also update `workspaces_test.go` to use `workspacePath` and remove the temporary `joinWorkspacePath` sentinel:

Replace
```go
func joinWorkspacePath(root string, n int) string {
	return filepath.Join(root, ".baag", "worktrees", "workspaceN")
}
```
with nothing — delete it.

And replace
```go
filepath.Join(wt, fmtWorkspaceName(n))
```
with
```go
workspacePath(root, n)
```

and delete `fmtWorkspaceName`, `itoa`, `fmtInt` — replace usage with `strconv.Itoa(n)` (add `"strconv"` to test imports). Test becomes simpler:

```go
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
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `go test ./... -run TestLowestFreeWorkspaceSlot -v`
Expected: all subtests PASS.

- [ ] **Step 5: Commit**

```bash
git add workspaces.go workspaces_test.go
git commit -m "feat: lowest-free workspace slot helper"
```

---

## Task 4: `provisionWorkspace` — fresh branch creation path

**Files:**
- Modify: `workspaces.go`
- Modify: `workspaces_test.go`

provision will call out to tmux too. For these tests we install a fake tmux that just records calls and exits 0.

- [ ] **Step 1: Add the failing test**

Append to `workspaces_test.go`:

```go
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
```

- [ ] **Step 2: Run test, verify it fails**

Run: `go test ./... -run TestProvisionWorkspaceCreatesWorktreeAndBranch -v`
Expected: FAIL — `undefined: provisionWorkspace`.

- [ ] **Step 3: Implement `provisionWorkspace`**

Append to `workspaces.go`:

```go
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
```

(Note: `os/exec` is already imported. Make sure `strings`, `os`, `filepath`, `fmt`, `strconv` are all in the import block.)

- [ ] **Step 4: Run test, verify it passes**

Run: `go test ./... -run TestProvisionWorkspaceCreatesWorktreeAndBranch -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add workspaces.go workspaces_test.go
git commit -m "feat: provisionWorkspace creates worktree + tmux session"
```

---

## Task 5: `provisionWorkspace` — existing branch reuse path

**Files:**
- Modify: `workspaces_test.go`

The reuse logic already exists in Task 4's code. This task adds the test that exercises it.

- [ ] **Step 1: Add the failing test**

Append to `workspaces_test.go`:

```go
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
```

- [ ] **Step 2: Run test, verify it passes**

Run: `go test ./... -run TestProvisionWorkspaceResetsExistingBranchToBase -v`
Expected: PASS (logic was already implemented in Task 4).

- [ ] **Step 3: Commit**

```bash
git add workspaces_test.go
git commit -m "test: cover branch-reuse path of provisionWorkspace"
```

---

## Task 6: `removeWorkspace` — happy path + safety check

**Files:**
- Modify: `workspaces.go`
- Modify: `workspaces_test.go`

- [ ] **Step 1: Add failing tests**

Append to `workspaces_test.go`:

```go
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
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `go test ./... -run TestRemoveWorkspace -v`
Expected: FAIL — `undefined: removeWorkspace`.

- [ ] **Step 3: Implement `removeWorkspace` (minimal — no dirty check yet)**

Append to `workspaces.go`:

```go
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

	worktreeArgs := []string{"-C", root, "worktree", "remove", path}
	if force {
		worktreeArgs = append(worktreeArgs[:4], "--force", path)
	}
	if out, err := exec.Command("git", worktreeArgs...).CombinedOutput(); err != nil {
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
		statePath, err := sessionStatePath(st.Name)
		if err == nil {
			_ = os.Remove(statePath)
		}
	}
	return nil
}
```

Make sure `isWorkspaceDir` is reachable here — it lives in `commands.go:660` already (same package).

- [ ] **Step 4: Run tests, verify they pass**

Run: `go test ./... -run TestRemoveWorkspace -v`
Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add workspaces.go workspaces_test.go
git commit -m "feat: removeWorkspace tears down worktree + branch + tmux state"
```

---

## Task 7: `removeWorkspace` — dirty refuses without force

**Files:**
- Modify: `workspaces_test.go`

The dirty logic is already in Task 6. This task adds the dirty/force tests.

- [ ] **Step 1: Add failing tests**

Append to `workspaces_test.go`:

```go
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
```

- [ ] **Step 2: Run tests, verify they pass**

Run: `go test ./... -run TestRemoveWorkspace -v`
Expected: all four `TestRemoveWorkspace*` subtests PASS.

- [ ] **Step 3: Commit**

```bash
git add workspaces_test.go
git commit -m "test: cover dirty/force paths of removeWorkspace"
```

---

## Task 8: `resetGitWorkspace` uses the detected base branch

**Files:**
- Modify: `commands.go:606-635`

Make `resetGitWorkspace` agree with provisionWorkspace by routing through `defaultBaseBranch`.

- [ ] **Step 1: Add a test that pins the new behaviour**

Append to `workspaces_test.go`:

```go
func TestResetGitWorkspaceUsesDetectedBase(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	// Switch default to `develop` to confirm we don't hard-code main.
	gitRun(t, root, "checkout", "-q", "-b", "develop")
	gitRun(t, root, "push", "-q", "-u", "origin", "develop")
	gitRun(t, root, "remote", "set-head", "origin", "develop")
	gitRun(t, root, "checkout", "-q", "main")

	// Advance develop on origin so the merge has something to bring in.
	clone := t.TempDir()
	gitRun(t, clone, "clone", "-q", gitOutput(t, root, "config", "--get", "remote.origin.url"), ".")
	gitRun(t, clone, "checkout", "-q", "develop")
	gitRun(t, clone, "commit", "--allow-empty", "-q", "-m", "advance develop")
	gitRun(t, clone, "push", "-q", "origin", "develop")

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
	// Sanity: the workspace was created from develop.
	if branch := gitOutput(t, res.Path, "rev-parse", "--abbrev-ref", "HEAD"); branch != "workspace-1" {
		t.Fatalf("workspace branch = %q", branch)
	}

	// Run reset (existing entrypoint).
	if err := resetGitWorkspace(res.Path, false); err != nil {
		t.Fatalf("resetGitWorkspace: %v", err)
	}

	// origin/develop's advance should now be merged in.
	logOut := gitOutput(t, res.Path, "log", "--oneline")
	if !strings.Contains(logOut, "advance develop") {
		t.Fatalf("expected advance develop merged into workspace, got:\n%s", logOut)
	}
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `go test ./... -run TestResetGitWorkspaceUsesDetectedBase -v`
Expected: FAIL — current `resetGitWorkspace` fetches `origin main`, which doesn't exist as the moving target here, OR merges the wrong branch.

- [ ] **Step 3: Modify `resetGitWorkspace` in `commands.go`**

Find (around line 606):

```go
func resetGitWorkspace(dir string, verbose bool) error {
	if !insideGitWorktree(dir) {
		if verbose {
			fmt.Println("Not a git repo, skipping git reset")
		}
		return nil
	}

	dirName := filepath.Base(dir)
	if isWorkspaceDir(dirName) {
		if verbose {
			fmt.Printf("Resetting worktree: %s\n", dirName)
		}
		if err := runGitCommand(dir, verbose, "checkout", dirName); err != nil {
			return err
		}
		if err := runGitCommand(dir, verbose, "fetch", "origin", "main"); err != nil {
			return err
		}
		return runGitCommand(dir, verbose, "merge", "origin/main", "-m", "Merge main into "+dirName)
	}

	if verbose {
		fmt.Printf("Resetting main directory: %s\n", dirName)
	}
	if err := runGitCommand(dir, verbose, "checkout", "main"); err != nil {
		return err
	}
	return runGitCommand(dir, verbose, "pull", "origin", "main")
}
```

Replace with:

```go
func resetGitWorkspace(dir string, verbose bool) error {
	if !insideGitWorktree(dir) {
		if verbose {
			fmt.Println("Not a git repo, skipping git reset")
		}
		return nil
	}

	base, err := defaultBaseBranch(dir)
	if err != nil {
		return err
	}

	dirName := filepath.Base(dir)
	if isWorkspaceDir(dirName) {
		if verbose {
			fmt.Printf("Resetting worktree: %s\n", dirName)
		}
		if err := runGitCommand(dir, verbose, "checkout", dirName); err != nil {
			return err
		}
		if err := runGitCommand(dir, verbose, "fetch", "origin", base); err != nil {
			return err
		}
		return runGitCommand(dir, verbose, "merge", "origin/"+base, "-m", "Merge "+base+" into "+dirName)
	}

	if verbose {
		fmt.Printf("Resetting main directory: %s\n", dirName)
	}
	if err := runGitCommand(dir, verbose, "checkout", base); err != nil {
		return err
	}
	return runGitCommand(dir, verbose, "pull", "origin", base)
}
```

- [ ] **Step 4: Run the test, verify it passes**

Run: `go test ./... -run TestResetGitWorkspaceUsesDetectedBase -v`
Expected: PASS.

- [ ] **Step 5: Run the full suite to catch regressions**

Run: `go test ./...`
Expected: all existing tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add commands.go workspaces_test.go
git commit -m "feat: resetGitWorkspace honors origin/HEAD instead of hardcoded main"
```

---

## Task 9: Picker — store `Root` per session

**Files:**
- Modify: `picker.go:31-42` (sessionInfo), `picker.go:gatherSessions`

We need to know each session's group-root in the picker so `+` can resolve where to provision.

- [ ] **Step 1: Add the field**

In `picker.go`, modify `sessionInfo` (line 31):

```go
type sessionInfo struct {
	Name    string
	Branch  string
	PR      *prInfo
	Claude  string
	Codex   string
	Server  bool
	Windows int
	Path    string
	Root    string // git common root (group-level), stripped of /.baag/worktrees/...
	Label   string
	Tasks   []taskInfo
}
```

- [ ] **Step 2: Populate it in `gatherSessions`**

In `gatherSessions` (around line 966 — the block that computes `group`), keep the existing group derivation, but also store the stripped root on `info.Root`:

Replace:

```go
		// Group by git root
		group := ""
		if info.Path != "" {
			rootOut, err := exec.Command("git", "-C", info.Path, "rev-parse", "--show-toplevel").Output()
			if err == nil {
				gitRoot := strings.TrimSpace(string(rootOut))
				if strings.Contains(gitRoot, "/.baag/worktrees/") {
					idx := strings.Index(gitRoot, "/.baag/worktrees/")
					group = filepath.Base(gitRoot[:idx])
				} else {
					group = filepath.Base(gitRoot)
				}
			}
		}
```

With:

```go
		// Group by git root
		group := ""
		if info.Path != "" {
			rootOut, err := exec.Command("git", "-C", info.Path, "rev-parse", "--show-toplevel").Output()
			if err == nil {
				gitRoot := strings.TrimSpace(string(rootOut))
				if idx := strings.Index(gitRoot, "/.baag/worktrees/"); idx >= 0 {
					info.Root = gitRoot[:idx]
				} else {
					info.Root = gitRoot
				}
				group = filepath.Base(info.Root)
			}
		}
```

- [ ] **Step 3: Run the build**

Run: `go build ./...`
Expected: succeeds.

Run: `go test ./...`
Expected: all PASS — this is a pure-additive change.

- [ ] **Step 4: Commit**

```bash
git add picker.go
git commit -m "feat: track per-session git root in picker"
```

---

## Task 10: Picker — `+` keybind dispatches provision

**Files:**
- Modify: `picker.go`
- Modify: `picker_test.go`

- [ ] **Step 1: Add failing test**

Append to `picker_test.go`:

```go
func TestPickerPlusDispatchesProvision(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "audrey-app",
			Root: "/tmp/audrey-app",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'+'}})
	if cmd == nil {
		t.Fatalf("+ should dispatch a cmd")
	}
	// The cmd is a tea.Cmd that runs provisionWorkspace; we just confirm the
	// status was set to indicate provisioning is in flight.
	// (Running the cmd would shell out to git, which we don't want in a unit test.)
}

func TestPickerPlusSetsProvisioningStatus(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "audrey-app",
			Root: "/tmp/audrey-app",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'+'}})
	pm := next.(pickerModel)
	if !strings.Contains(pm.status, "provisioning") {
		t.Fatalf("status = %q, want it to mention provisioning", pm.status)
	}
	if pm.statusErr {
		t.Fatalf("status should not be flagged as error")
	}
}

func TestPickerPlusIgnoresRowWithoutRoot(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "x"},
		{Kind: rowSession, Group: "x", Session: &sessionInfo{Name: "x"}}, // no Root
	})
	time.Sleep(200 * time.Millisecond)

	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'+'}})
	if cmd != nil {
		t.Fatalf("expected no cmd when row has no Root")
	}
	pm := next.(pickerModel)
	if !pm.statusErr {
		t.Fatalf("expected error status, got %q", pm.status)
	}
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `go test ./... -run TestPickerPlus -v`
Expected: FAIL — `+` currently triggers the auto-filter branch.

- [ ] **Step 3: Wire `+` in `Update`**

In `picker.go`, find the main keybind switch (around line 433, after `case "G":` block) and add a case BEFORE the catch-all `default:` clause:

```go
		case "+":
			return m, m.provisionInFocusedGroup()
		case "D":
			return m, m.deleteSelectedWorkspace()
```

Also reuse `pickerActionMsg` — add the new actions to `applyPickerAction` (we'll fill in delete in Task 11). For now, add:

```go
func (m *pickerModel) provisionInFocusedGroup() tea.Cmd {
	session := m.selectedSession()
	if session == nil || session.Root == "" {
		m.status = "no git root for this row"
		m.statusErr = true
		return nil
	}
	root := session.Root
	group := m.rows[m.visible[m.cursor]].Group
	m.status = fmt.Sprintf("provisioning new workspace in %s", group)
	m.statusErr = false
	return func() tea.Msg {
		res, err := provisionWorkspace(root)
		return pickerActionMsg{
			Action:  "provision",
			Session: res.Session,
			Value:   res.Branch,
			Err:     err,
		}
	}
}
```

Add a placeholder delete to satisfy the compiler:

```go
func (m *pickerModel) deleteSelectedWorkspace() tea.Cmd {
	// Real implementation lands in Task 11.
	return nil
}
```

And extend `applyPickerAction` to handle the provision result. Add to the switch:

```go
		case "provision":
			if msg.Err != nil {
				m.status = fmt.Sprintf("provision failed: %v", msg.Err)
				m.statusErr = true
				return
			}
			m.status = fmt.Sprintf("provisioned %s", msg.Value)
			m.statusErr = false
```

Note: the existing error switch at the top of `applyPickerAction` also branches on `msg.Action`. Add a case there too so the error path renders cleanly:

```go
		case "provision":
			m.status = fmt.Sprintf("provision failed: %v", msg.Err)
```

- [ ] **Step 4: Run the picker tests**

Run: `go test ./... -run TestPickerPlus -v`
Expected: all three PASS.

- [ ] **Step 5: Make sure `+` doesn't break the auto-filter path**

The catch-all `default:` clause at the bottom of the keybind switch starts filtering when an arbitrary ASCII key is pressed. Our explicit `+` case now intercepts before it — good. Verify no test that types `+` into filter mode regresses:

Run: `go test ./... -v` (full suite)
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: + key provisions workspace in focused group"
```

---

## Task 11: Picker — `D` keybind with y/N confirmation

**Files:**
- Modify: `picker.go`
- Modify: `picker_test.go`

- [ ] **Step 1: Add failing tests**

Append to `picker_test.go`:

```go
func TestPickerDeleteEntersConfirmMode(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "workspace-1",
			Path: "/r/.baag/worktrees/workspace-1",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)
	if !pm.confirmDelete {
		t.Fatalf("D should enter confirmDelete mode")
	}
	if pm.deleteSlot != 1 {
		t.Fatalf("deleteSlot = %d, want 1", pm.deleteSlot)
	}
}

func TestPickerDeleteRefusesMainRow(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "audrey-app",
			Path: "/r",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)
	if pm.confirmDelete {
		t.Fatalf("D on main row should not enter confirmDelete")
	}
	if !pm.statusErr {
		t.Fatalf("expected error status, got %q", pm.status)
	}
}

func TestPickerDeleteCancelOnAnyOtherKey(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name: "workspace-1",
			Path: "/r/.baag/worktrees/workspace-1",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)

	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	pm = next.(pickerModel)
	if pm.confirmDelete {
		t.Fatalf("non-y key should cancel confirmDelete")
	}
}

func TestPickerDeleteYDispatchesRemove(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name: "workspace-1",
			Path: "/r/.baag/worktrees/workspace-1",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)

	next, cmd := pm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	pm = next.(pickerModel)
	if pm.confirmDelete {
		t.Fatalf("y should exit confirmDelete mode")
	}
	if cmd == nil {
		t.Fatalf("y should dispatch a remove cmd")
	}
	if !strings.Contains(pm.status, "removing") {
		t.Fatalf("status = %q, want it to mention removing", pm.status)
	}
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `go test ./... -run TestPickerDelete -v`
Expected: FAIL — `confirmDelete`/`deleteSlot` undefined and `D` does nothing useful yet.

- [ ] **Step 3: Add state + handler in `picker.go`**

Extend `pickerModel`:

```go
type pickerModel struct {
	rows         []pickerRow
	visible      []int
	cursor       int
	filter       textinput.Model
	filtering    bool
	labelInput   textinput.Model
	labelEditing bool
	labelTarget  string
	confirmDelete bool
	deleteSlot    int
	deleteRoot    string
	deleteBranch  string
	deleteForce   bool
	showTasks    bool
	status       string
	statusErr    bool
	width        int
	height       int
	startedAt    time.Time
	spinnerFrame int
}
```

Replace the placeholder from Task 10:

```go
func (m *pickerModel) deleteSelectedWorkspace() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	if session.Root == "" {
		m.status = "no git root for this row"
		m.statusErr = true
		return nil
	}
	dir := filepath.Base(session.Path)
	if !isWorkspaceDir(dir) {
		m.status = "cannot delete main worktree"
		m.statusErr = true
		return nil
	}
	slot, err := strconv.Atoi(strings.TrimPrefix(dir, "workspace-"))
	if err != nil || slot < 1 {
		m.status = fmt.Sprintf("unrecognised workspace dir: %s", dir)
		m.statusErr = true
		return nil
	}
	m.confirmDelete = true
	m.deleteSlot = slot
	m.deleteRoot = session.Root
	m.deleteBranch = dir
	m.deleteForce = false
	return nil
}
```

Add `"strconv"` to picker.go's import block.

Add a handler block at the top of `Update`'s `case tea.KeyMsg:` (BEFORE the `labelEditing` block):

```go
			if m.confirmDelete {
				switch key {
				case "ctrl+c":
					return m, tea.Quit
				case "y", "Y":
					m.confirmDelete = false
					target := m.deleteBranch
					root := m.deleteRoot
					slot := m.deleteSlot
					force := m.deleteForce
					m.status = fmt.Sprintf("removing %s", target)
					m.statusErr = false
					return m, func() tea.Msg {
						return pickerActionMsg{
							Action:  "delete",
							Session: target,
							Value:   strconv.Itoa(slot),
							Err:     removeWorkspace(root, slot, force),
						}
					}
				default:
					m.confirmDelete = false
					m.status = "delete cancelled"
					return m, nil
				}
			}
```

Extend `applyPickerAction`'s success switch:

```go
		case "delete":
			if msg.Err == errDirtyWorkspace {
				m.confirmDelete = true
				m.deleteForce = true
				m.status = fmt.Sprintf("%s has unpushed work — force delete? (y/N)", msg.Session)
				m.statusErr = true
				return
			}
			// Drop the row from view; refresh tick will reconcile fully.
			for i, row := range m.rows {
				if row.Session != nil && row.Session.Name == msg.Session {
					m.rows = append(m.rows[:i], m.rows[i+1:]...)
					m.rebuildVisible()
					m.clampCursor()
					break
				}
			}
			m.status = fmt.Sprintf("removed %s", msg.Session)
			m.statusErr = false
```

Extend the error switch:

```go
		case "delete":
			m.status = fmt.Sprintf("delete %s failed: %v", msg.Session, msg.Err)
```

Note: `errDirtyWorkspace` arrives as the `Err` field. The error switch runs unconditionally when `msg.Err != nil`, which would shadow the re-prompt path. Restructure `applyPickerAction` to special-case the dirty error before the generic error handler. Replace the top of `applyPickerAction`:

```go
func (m *pickerModel) applyPickerAction(msg pickerActionMsg) {
	if msg.Err == errDirtyWorkspace && msg.Action == "delete" {
		m.confirmDelete = true
		m.deleteForce = true
		m.status = fmt.Sprintf("%s has unpushed work — force delete? (y/N)", msg.Session)
		m.statusErr = true
		return
	}
	if msg.Err != nil {
		// ...existing error switch as before...
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `go test ./... -run TestPickerDelete -v`
Expected: all four PASS.

- [ ] **Step 5: Run the full suite**

Run: `go test ./...`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: D key removes workspace with y/N + force re-prompt"
```

---

## Task 12: Picker — main-worktree marker glyph

**Files:**
- Modify: `picker.go:renderSession`
- Modify: `picker_test.go`

- [ ] **Step 1: Add failing test**

Append to `picker_test.go`:

```go
func TestRenderSessionMarksMainWorktree(t *testing.T) {
	mainRow := pickerRow{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
		Name: "audrey-app",
		Path: "/r/audrey-app",
		Root: "/r/audrey-app",
	}}
	wsRow := pickerRow{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
		Name: "workspace-1",
		Path: "/r/audrey-app/.baag/worktrees/workspace-1",
		Root: "/r/audrey-app",
	}}
	m := newPickerModel([]pickerRow{mainRow, wsRow})

	mainOut := m.renderSession(mainRow, false)
	wsOut := m.renderSession(wsRow, false)

	if !strings.Contains(mainOut, "◇") {
		t.Fatalf("main row missing ◇ glyph:\n%s", mainOut)
	}
	if strings.Contains(wsOut, "◇") {
		t.Fatalf("workspace row should not have ◇ glyph:\n%s", wsOut)
	}
}
```

- [ ] **Step 2: Run, verify it fails**

Run: `go test ./... -run TestRenderSessionMarksMainWorktree -v`
Expected: FAIL.

- [ ] **Step 3: Implement the glyph**

In `picker.go`, add a helper at the top of the file (near `isSelectablePickerRow`):

```go
// isMainWorktreePath returns true when path looks like the main checkout of
// a project (i.e. not nested under /.baag/worktrees/).
func isMainWorktreePath(path string) bool {
	if path == "" {
		return false
	}
	return !strings.Contains(path, "/.baag/worktrees/")
}
```

Add a glyph style with the existing styles:

```go
	pMainMark = lipgloss.NewStyle().
		Foreground(overlay0)
```

In `renderSession`, after computing `waiting`/`active` and BEFORE the prefix block, decide whether to render the marker. Replace the prefix block:

```go
	// Prefix: left accent bar for waiting, cursor arrow for selected
	if waiting {
		if selected {
			line.WriteString("  " + pWaitingDot.Render("▎") + pCursor.Render("›") + " ")
		} else {
			line.WriteString("  " + pWaitingDot.Render("▎") + "  ")
		}
	} else if selected {
		line.WriteString("   " + pCursor.Render("›") + " ")
	} else {
		line.WriteString("     ")
	}
```

With (note: keep total leading width identical at 5 columns):

```go
	mainGlyph := " "
	if isMainWorktreePath(s.Path) {
		mainGlyph = pMainMark.Render("◇")
	}
	// Prefix: left accent bar for waiting, cursor arrow for selected, then
	// the main-worktree marker. Always 5 columns to keep alignment.
	switch {
	case waiting && selected:
		line.WriteString("  " + pWaitingDot.Render("▎") + pCursor.Render("›") + mainGlyph)
	case waiting:
		line.WriteString("  " + pWaitingDot.Render("▎") + " " + mainGlyph)
	case selected:
		line.WriteString("   " + pCursor.Render("›") + mainGlyph)
	default:
		line.WriteString("    " + mainGlyph)
	}
	line.WriteString(" ")
```

- [ ] **Step 4: Run, verify it passes**

Run: `go test ./... -run TestRenderSessionMarksMainWorktree -v`
Expected: PASS.

- [ ] **Step 5: Verify other picker view tests still pass (alignment regression check)**

Run: `go test ./... -run TestPickerView -v`
Expected: PASS (height-fits test in particular).

- [ ] **Step 6: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: subtle ◇ marker for each group's main worktree"
```

---

## Task 13: Footer — `+ new` and `D delete`

**Files:**
- Modify: `picker.go:View` (footer block)

- [ ] **Step 1: Update the footer**

In `picker.go`, find the footer block around line 765:

```go
	b.WriteString("    " +
		pFooterKey.Render("↑↓") + pFooter.Render(" navigate") + sep +
		pFooterKey.Render("⏎") + pFooter.Render(" switch") + sep +
		pFooterKey.Render("n") + pFooter.Render(" name") + sep +
		pFooterKey.Render("c") + pFooter.Render(" clear") + sep +
		pFooterKey.Render("s") + pFooter.Render(" server") + sep +
		pFooterKey.Render("tab") + pFooter.Render(" "+todoLabel) + sep +
		pFooterKey.Render("/") + pFooter.Render(" filter") + sep +
		pFooterKey.Render("esc") + pFooter.Render(" close"))
```

Replace with:

```go
	b.WriteString("    " +
		pFooterKey.Render("↑↓") + pFooter.Render(" navigate") + sep +
		pFooterKey.Render("⏎") + pFooter.Render(" switch") + sep +
		pFooterKey.Render("+") + pFooter.Render(" new") + sep +
		pFooterKey.Render("D") + pFooter.Render(" delete") + sep +
		pFooterKey.Render("n") + pFooter.Render(" name") + sep +
		pFooterKey.Render("c") + pFooter.Render(" clear") + sep +
		pFooterKey.Render("s") + pFooter.Render(" server") + sep +
		pFooterKey.Render("tab") + pFooter.Render(" "+todoLabel) + sep +
		pFooterKey.Render("/") + pFooter.Render(" filter") + sep +
		pFooterKey.Render("esc") + pFooter.Render(" close"))
```

- [ ] **Step 2: Sanity-check no regression**

Run: `go test ./...`
Expected: PASS (including the height-fits test, which already accounts for the footer line on a single row).

- [ ] **Step 3: Commit**

```bash
git add picker.go
git commit -m "feat: footer hints for + new / D delete"
```

---

## Task 14: Manual smoke + verification

**Files:** none

- [ ] **Step 1: Build the binary**

Run: `go build`
Expected: builds, no errors.

- [ ] **Step 2: `go vet`**

Run: `go vet ./...`
Expected: no output.

- [ ] **Step 3: Full test pass**

Run: `go test ./...`
Expected: all PASS.

- [ ] **Step 4: Eyeball the picker in a real tmux session (manual)**

Run: `./domux sessions`

Eyeball checks:
- Each group shows `◇` next to the main checkout row.
- Footer includes `+ new`, `D delete`.
- Press `+` on an `audrey-app` row → status flips to `provisioning new workspace in audrey-app`. (Don't actually fire if you don't want a new workspace — Ctrl+C exits before completion.)
- Press `D` on a `workspace-N` row → footer status shows the confirmation.
- Press `D` on a main-worktree row → status says `cannot delete main worktree`, no mode change.

If anything in the manual smoke is off, fix inline and commit before moving on.

---

## Self-review (run after writing all tasks)

**Spec coverage check:**

| Spec section | Implemented in |
|---|---|
| Architecture: new `workspaces.go` with named API | Tasks 2-7 |
| `provisionWorkspace` | Tasks 4, 5 |
| `removeWorkspace` (dirty + force) | Tasks 6, 7 |
| `defaultBaseBranch` | Task 2 |
| `resetGitWorkspace` uses default base | Task 8 |
| Picker `+` keybind | Task 10 |
| Picker `D` keybind + y/N + force re-prompt | Task 11 |
| Main-worktree marker glyph | Task 12 |
| Footer hints | Task 13 |
| Tests for default-base / slot / provision / remove | Tasks 2-7 |
| Tests for picker behaviour | Tasks 10-12 |

**Placeholder scan:** none — every step has runnable code or shell commands.

**Type consistency:** `workspaceResult`, `provisionWorkspace(root)`, `removeWorkspace(root, slot, force)`, `errDirtyWorkspace`, `workspacePath`, `workspaceBranch`, `defaultBaseBranch` — all used consistently across plan and matching the spec.
