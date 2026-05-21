# Workspace provisioning in the domux switcher

**Date:** 2026-05-21
**Status:** Spec — pending implementation

## Goal

Let the user create, remove, and visually identify `workspace-N` worktrees from inside the domux switcher TUI, replacing manual use of `baag` for the common "give me another scratch worktree on this project" loop.

## Non-goals

- Named (non-numbered) worktrees. Scope is numbered slots only.
- Provisioning outside the picker. No interactive prompt-driven CLI command beyond what the picker uses.
- Replacing `baag` for feature-branch worktrees (the `feature/...` named ones in `.baag/worktrees/`). Those stay baag-managed.
- Cross-project workspace creation. The picker only adds workspaces to a group that already has at least one session.

## Background

`audrey-app` already follows the convention this feature standardizes on:

- Primary checkout at `<project>/`, on the default branch (`main`).
- Sibling worktrees at `<project>/.baag/worktrees/workspace-N`, each on a same-named branch (`workspace-N`), branched from `origin/main`.
- The `workspace-N` branch is treated as that worktree's "main": `domux clear` already merges `origin/main` into it (`resetGitWorkspace` in `commands.go`).

`isWorkspaceDir` (`commands.go:660`) already recognizes the naming convention. The picker (`picker.go:gatherSessions`) already groups by stripping `/.baag/worktrees/...` from the git root.

What's missing: a way to actually create and tear down these worktrees from the switcher.

## Decisions made during brainstorming

| Decision | Choice |
|---|---|
| Worktree scope | Only numbered `workspace-N` slots |
| Add trigger | Keybind `+`, adds to the focused group |
| Remove trigger | Keybind `D` on `workspace-N` row, y/N confirm |
| Slot numbering | Lowest free N |
| Branch reuse | Force-reset existing `workspace-N` branch to default base |
| Base branch | Auto-detect via `git symbolic-ref refs/remotes/origin/HEAD`, fallback `main` |
| Implementation | Native via `exec.Command` (no `baag` dependency) |

## Architecture

One new file: `workspaces.go`. Public API:

```go
type workspaceResult struct {
    Path       string
    Branch     string
    Session    string
    BaseBranch string
}

func provisionWorkspace(root string) (workspaceResult, error)
func removeWorkspace(root string, slot int, force bool) error
func defaultBaseBranch(root string) (string, error)
func isMainWorktreePath(group, path string) bool
```

Modifications:
- `picker.go` — new `+` and `D` keybinds, a `confirmDelete` mode mirroring `labelEditing`, and a subtle marker glyph for the main worktree row in each group.
- `commands.go` — `resetGitWorkspace` switches its hardcoded `"main"` to `defaultBaseBranch(dir)`, keeping reset and add consistent.
- `main.go`/`commands.go` — optional `domux workspace add|remove` subcommand mostly for testing; the picker drives the real UX.

`workspaces.go` shells out to `git` and `tmux` directly. No new dependencies.

## Picker UX

### Add (`+`)

1. User presses `+` from any state (not in filter/label mode).
2. Picker reads the group of the focused row. If no row is focused or the focused row isn't in a git-backed group, status: `not a git repo`.
3. Picker dispatches a tea.Cmd that runs `provisionWorkspace(root)`.
4. Status footer shows `provisioning workspace-3 in audrey-app…` (yellow).
5. On success: status flips green `workspace-3 ready`. Picker refresh picks up the new row; cursor jumps to it; `tea.Quit` attaches.
6. On failure: status flips red with the git error.

### Remove (`D`)

1. Valid only on rows where `isWorkspaceDir(filepath.Base(session.Path))` is true. Otherwise status: `cannot delete main worktree` (or for non-workspace rows, `D` is a no-op).
2. Picker enters `confirmDelete` mode (parallel to `labelEditing`). Footer renders `delete workspace-3? (y/N)` in red.
3. `y` triggers `removeWorkspace(root, n, false)`. Any other key cancels.
4. If `removeWorkspace` returns `errDirtyWorkspace`, footer re-prompts: `workspace-3 has unpushed/uncommitted work — force delete? (y/N)`. `y` calls `removeWorkspace(root, n, true)`.
5. On success: status `removed workspace-3`, refresh removes the row.

### Main-worktree marker

In each group, the "main" row is the one whose path is NOT under `/.baag/worktrees/` — same predicate the picker already uses for group extraction (`gatherSessions` in `picker.go:967`). That row gets a single `◇` glyph in `overlay0` rendered between the cursor space and the name:

```
AUDREY-APP ───────────────────
   ◇ audrey-app · main
     workspace-1 · workspace-1
     workspace-2 · workspace-2
```

Width-neutral: replaces one of the leading spaces, doesn't push other rows. No additional color on the row name — the glyph alone signals "this is the canonical checkout."

### Footer

Add `+ new` and `D delete` to the footer. `c clear` (reset) stays — reset and delete are distinct verbs.

## Lifecycle

### Provision

```
provisionWorkspace(root) →
  base       = defaultBaseBranch(root)                    // origin/main, fallback main
  git fetch origin <base>
  slot       = lowestFreeSlot(root)                       // lowest N with no dir at path AND no entry in `git worktree list` for that path
  branch     = "workspace-<slot>"
  path       = root/.baag/worktrees/workspace-<slot>
  mkdir -p   root/.baag/worktrees
  if branch exists:
      git branch -f <branch> origin/<base>                // reset to base
      git worktree add <path> <branch>
  else:
      git worktree add -b <branch> <path> origin/<base>
  session = uniqueSessionName("workspace-<slot>")         // collision suffix -2, -3…
  tmux new-session -d -s <session> -c <path>
  setSessionRoot(session, path)
  attachTmuxSession(session)
```

### Remove

```
removeWorkspace(root, slot, force) →
  branch = "workspace-<slot>"
  path   = root/.baag/worktrees/workspace-<slot>
  if filepath.Base(path) is not a workspaceDir → return error
  if !force:
      if git -C path status --porcelain non-empty → errDirtyWorkspace
      if git -C path log @{u}..HEAD non-empty   → errDirtyWorkspace  // unpushed
  for each domux session with state.Root == path:
      tmux kill-session -t <name>
      remove ~/.local/share/domux/sessions/<name>.json
      clearSessionStateFiles(home, name)                  // legacy .tmux-* files
  git -C root worktree remove [--force] <path>
  git -C root branch -D <branch>
```

The unpushed check: if `git rev-parse @{u}` resolves, use `git log @{u}..HEAD --oneline`. Otherwise (no upstream — branch never pushed), use `git log origin/<base>..HEAD --oneline`. Either non-empty result → `errDirtyWorkspace`.

### Reset (existing, modified)

`resetGitWorkspace` (`commands.go:606`) becomes:

```go
base, err := defaultBaseBranch(dir)
// existing logic, with "main" → base
```

Both the workspace-dir branch (merges `origin/<base>` in) and the main-dir branch (`git checkout <base>; git pull`) use the detected base.

## Error handling

- Each git/tmux command surfaces its stderr through the returned error. Picker renders via the existing `pStatusErr` style.
- No partial-state rollback — git's atomicity handles the worktree case; orphaned tmux sessions or state files from a half-finished provision are visible on refresh and the user can re-press `+`.
- `defaultBaseBranch` failure (no `origin/HEAD`, not a repo) → returns error; picker shows hint: `set git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main`.
- `D` on a main-worktree row: status-line no-op `cannot delete main worktree`. Never enters confirm mode.
- Concurrent `+` presses: synchronous tea.Cmd; second one would see "directory exists" from `git worktree add` and surface it.

## Testing

`workspaces_test.go` (new):

- `TestDefaultBaseBranch_FromSymbolicRef` — repo with `refs/remotes/origin/HEAD` → returns symbolic target.
- `TestDefaultBaseBranch_Fallback` — no symbolic ref → returns `"main"`.
- `TestPickLowestFreeWorkspaceSlot` — table: empty → 1; `[1,2]` → 3; `[1,3]` → 2; `[2]` → 1.
- `TestProvisionWorkspace_CreatesWorktreeAndBranch` — real git in `t.TempDir()`, tmux mocked via `PATH` shim (existing pattern, see `commands_test.go`). Asserts dir exists, branch points at base.
- `TestProvisionWorkspace_ResetsExistingBranch` — pre-create `workspace-1` ahead of base → after provision, branch sits at base.
- `TestRemoveWorkspace_RefusesDirtyWithoutForce` — provision, dirty the worktree, expect `errDirtyWorkspace`.
- `TestRemoveWorkspace_RefusesUnpushedWithoutForce` — provision, commit, no upstream → `errDirtyWorkspace`.
- `TestRemoveWorkspace_ForceRemoves` — same setup, `force=true`, expect success and worktree gone.
- `TestRemoveWorkspace_RefusesNonWorkspaceDir` — point at a non-`workspace-N` dir → error, nothing removed.

`picker_test.go` additions:

- `TestPickerAddKeyRoutesToFocusedGroup` — render rows, focus a session in group `audrey-app`, fire `+`, assert resulting action carries that root.
- `TestPickerDeleteRequiresConfirmation` — `D` on workspace-N row enters confirmDelete; arbitrary key cancels; `y` dispatches remove.
- `TestPickerDeleteSkippedOnMainRow` — `D` on main row → status set, no confirm mode entered.
- `TestRenderSessionMarksMainWorktree` — assert `◇` glyph on the main row, not on workspace-N rows.

`commands_test.go` adjustment:

- Update any test that locks `resetGitWorkspace` to literal `main` so it reads `defaultBaseBranch` (or shim it via the test repo's `origin/HEAD`).

## Open questions

None remaining. All forks resolved during brainstorming.

## Out of scope (parked for later)

- Named worktrees (`baag start <name>` equivalent) from the picker.
- Workspace provisioning for groups with no current session (would need a "new project" path in the picker first).
- Per-project configurable base branch override beyond `origin/HEAD`.
