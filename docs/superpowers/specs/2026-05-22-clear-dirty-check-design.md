# Design: clear dirty-check + hard reset for workspace worktrees

## Problem

`domux clear` from the session picker runs `resetGitWorkspace` on the workspace path, but does no pre-flight dirty check. If the workspace has uncommitted changes the git commands fail with cryptic errors. The reset also does a merge instead of a hard reset, so local commits on the workspace branch survive — which defeats the "start clean" intent.

## Goal

1. Block clear when the workspace has uncommitted changes; show a human-readable status error in the picker.
2. For workspace-N dirs, replace the fetch+merge with fetch + `reset --hard origin/<base>`.
3. Non-workspace dirs (main worktrees) keep existing behavior (checkout + pull).

## Changes

### `commands.go`

**New sentinel:**
```go
var errClearDirty = errors.New("uncommitted changes — commit or stash first")
```

**`resetGitWorkspace` changes:**

1. After `insideGitWorktree` check, add:
   ```go
   statusOut, err := exec.Command("git", "-C", dir, "status", "--porcelain").Output()
   if err != nil {
       return fmt.Errorf("git status: %w", err)
   }
   if len(strings.TrimSpace(string(statusOut))) > 0 {
       return errClearDirty
   }
   ```

2. Workspace branch reset changes from:
   ```go
   git checkout <dirName>
   git fetch origin <base>
   git merge origin/<base> -m "..."
   ```
   to:
   ```go
   git fetch origin <base>
   git reset --hard origin/<base>
   ```
   (The explicit `checkout` is dropped — the worktree is already on `workspace-N`.)

### `picker.go`

**`applyPickerAction`** — add before the generic `msg.Err != nil` block:
```go
if msg.Err == errClearDirty && msg.Action == "clear" {
    m.status = fmt.Sprintf("%s has uncommitted changes — commit or stash first", msg.Session)
    m.statusErr = true
    return
}
```

## Error handling

| Scenario | Behavior |
|---|---|
| Uncommitted changes | `errClearDirty` → picker shows status error, no action taken |
| Not a git repo | unchanged — silent skip |
| `git fetch` / `git reset` fails | hard error shown in picker status bar (existing path) |
| Non-workspace dir | unchanged — checkout + pull |

## Non-goals

- No force-clear prompt (unlike delete's force flow) — clear with dirty state is always rejected.
- Unpushed commits are NOT checked — only `--porcelain` uncommitted changes.
