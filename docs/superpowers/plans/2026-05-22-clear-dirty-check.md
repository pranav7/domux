# Clear Dirty-Check + Hard Reset Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Block `domux clear` when the workspace has uncommitted changes, show a picker status error, and replace the workspace branch merge with a hard reset to `origin/<base>`.

**Architecture:** Add `errClearDirty` sentinel in `commands.go`; modify `resetGitWorkspace` to check porcelain status then do `fetch + reset --hard`; handle the sentinel in `picker.go`'s `applyPickerAction`.

**Tech Stack:** Go 1.22, bubbletea, exec.Command git calls

---

### Task 1: Add dirty check + hard reset to `resetGitWorkspace`

**Files:**
- Modify: `commands.go` (functions `resetGitWorkspace` ~line 606, add sentinel ~line 465)

- [ ] **Step 1: Write failing test for dirty-check rejection**

In `workspaces_test.go`, add after the existing `TestResetGitWorkspaceUsesDetectedBase` test:

```go
func TestResetGitWorkspaceRejectsDirty(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	// Dirty the working tree.
	if err := os.WriteFile(filepath.Join(root, "dirty.txt"), []byte("x"), 0644); err != nil {
		t.Fatal(err)
	}
	err := resetGitWorkspace(root, false)
	if err != errClearDirty {
		t.Fatalf("err = %v, want errClearDirty", err)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

```
go test -run TestResetGitWorkspaceRejectsDirty ./...
```

Expected: FAIL — `errClearDirty` undefined.

- [ ] **Step 3: Add sentinel and dirty check to `commands.go`**

Add the sentinel near the top of the error vars (around line 25, after existing `var` blocks):

```go
var errClearDirty = errors.New("uncommitted changes — commit or stash first")
```

Then in `resetGitWorkspace`, replace:

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

with:

```go
func resetGitWorkspace(dir string, verbose bool) error {
	if !insideGitWorktree(dir) {
		if verbose {
			fmt.Println("Not a git repo, skipping git reset")
		}
		return nil
	}

	statusOut, err := exec.Command("git", "-C", dir, "status", "--porcelain").Output()
	if err != nil {
		return fmt.Errorf("git status: %w", err)
	}
	if len(strings.TrimSpace(string(statusOut))) > 0 {
		return errClearDirty
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
		if err := runGitCommand(dir, verbose, "fetch", "origin", base); err != nil {
			return err
		}
		return runGitCommand(dir, verbose, "reset", "--hard", "origin/"+base)
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

- [ ] **Step 4: Run test to verify it passes**

```
go test -run TestResetGitWorkspaceRejectsDirty ./...
```

Expected: PASS.

- [ ] **Step 5: Run full test suite**

```
go test ./...
```

Expected: all pass (existing `TestResetGitWorkspaceUsesDetectedBase` may need network — skip if needed).

- [ ] **Step 6: Commit**

```
git add commands.go workspaces_test.go
git commit -m "feat: dirty check + hard reset in resetGitWorkspace"
```

---

### Task 2: Handle `errClearDirty` in the session picker

**Files:**
- Modify: `picker.go` (function `applyPickerAction` ~line 708)

- [ ] **Step 1: Add picker handling for errClearDirty**

In `picker.go`, in `applyPickerAction`, the first block currently is:

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
```

Change to:

```go
func (m *pickerModel) applyPickerAction(msg pickerActionMsg) {
	if msg.Err == errDirtyWorkspace && msg.Action == "delete" {
		m.confirmDelete = true
		m.deleteForce = true
		m.status = fmt.Sprintf("%s has unpushed work — force delete? (y/N)", msg.Session)
		m.statusErr = true
		return
	}
	if msg.Err == errClearDirty && msg.Action == "clear" {
		m.status = fmt.Sprintf("%s has uncommitted changes — commit or stash first", msg.Session)
		m.statusErr = true
		return
	}
	if msg.Err != nil {
```

- [ ] **Step 2: Build to verify no compile errors**

```
go build ./...
```

Expected: exits 0.

- [ ] **Step 3: Run full test suite**

```
go test ./...
```

Expected: all pass.

- [ ] **Step 4: Commit**

```
git add picker.go
git commit -m "feat: show errClearDirty as status error in picker"
```
