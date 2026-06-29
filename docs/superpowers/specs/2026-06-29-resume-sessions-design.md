# Resuming saved domux sessions after a restart

**Date:** 2026-06-29
**Status:** Spec — pending implementation

## Goal

Add `domux resume` so that, after a machine restart (or any time the tmux server is gone), the user can recreate their tmux work sessions from domux's already-persisted session state — no manual `tmux new-session` per worktree. One command brings the whole workspace back; an optional project argument narrows it to one group. The user lands in the switcher and watches sessions repopulate.

## Non-goals

- Remembering anything new. Session state files (`~/.local/share/domux/sessions/*.json`) already record `Name` + `Root` and survive reboots. This feature *reads* them; it adds no new persistence.
- Re-running worktree setup. The `.domux/worktree.conf` `run` directives exist to scaffold a *brand-new* worktree (install deps, copy env, start watchers). On resume the worktree already exists fully set up on disk — re-running setup would be redundant at best, clobber files or spawn duplicate processes at worst. Resume recreates the bare shell session only.
- Relaunching Claude/Codex or dev servers. The user restarts those themselves.
- Restoring tmux window/pane layouts. A recreated session is a single window with a shell at the saved root, exactly like `createTmuxSession` produces today.

## Background

The data needed to resume already exists and already persists across reboots:

- `~/.local/share/domux/sessions/<session>.json` holds a `SessionState` with `Name` and `Root` (the worktree path), plus label, server flag, AI map, etc. (`session.go`).
- All of that metadata **reattaches automatically** once a tmux session with the saved `Name` exists again, because everything keys off the session name: `loadSessionStateWithLegacy(name)` in `gatherSessions` rehydrates label, todos (via `Root`→`TodoPath`), recap, and server flag with no extra work.

What a restart destroys is the **tmux server**. With it gone, `tmux list-sessions` returns empty, `gatherSessions()` produces nothing, and the picker shows an empty screen even though every state file is intact. The saved states become orphans pointing at sessions that no longer exist.

So "resume" reduces to: read the persisted states, and for each one recreate a detached tmux session at its saved `Root`. The existing picker refresh loop then fills in the rest.

Relevant existing pieces:
- `listSessionStates()` (`session.go`) — reads every `*.json` state.
- `createTmuxSession(name, root)` / `tmuxSessionExists(name)` (`commands.go`) — recreate / probe. `tmux new-session` transparently starts the server if it's down.
- `setSessionRoot(name, root)` (`session.go`) — pins `Root`/`TodoPath` on the state.
- `removeSessionState` + `clearSessionStateFiles` (`session.go`/`commands.go`) — drop a state file and its legacy `.tmux-*` residue.
- `workspaceRootFromPath(path)` (`workspaces.go`) — strips a `.domux/worktrees/workspace-N` suffix back to the main checkout. The picker's group key is `filepath.Base` of this. Resume reuses it for group matching.
- The picker's `m.status` + 5s TTL machinery and `logoHeaderLines` status box (`picker.go`) — reused to narrate progress.

## Decisions made during brainstorming

| Decision | Choice |
|---|---|
| Command surface | `domux resume` (everything) and `domux resume <project>` (one group) |
| After recreate | Recreate detached, then land the user in the switcher; progress shown inside it |
| Restore depth | Bare shell session at saved root only — no setup, no agents, no servers |
| Stale state (root gone) | Skip **and** prune the orphaned state file |
| Recreate order | Sequential, one session per UI message round-trip |
| Progress rendering | Header status line + the self-populating live list; no separate full-screen panel |

## Architecture

One new file: `resume.go`. The planning half is pure and unit-testable; the execution half shells out.

```go
type resumeStatus string

const (
    resumeRecreated resumeStatus = "recreated"
    resumeRunning   resumeStatus = "already running" // session already exists
    resumePruned    resumeStatus = "pruned"          // root gone → state file removed
)

type resumeTarget struct {
    Name   string
    Root   string
    Group  string       // filepath.Base(workspaceRootFromPath(Root) | Root)
    Status resumeStatus // set during execution
    Err    error
}

// planResume is pure: given all saved states and an optional group filter,
// it partitions them into sessions to recreate vs. state files to prune.
// A state whose Root directory is missing on disk goes to prune. When
// filter != "", only states whose group == filter are considered (others
// are ignored entirely — neither recreated nor pruned).
func planResume(states []SessionState, filter string) (recreate []resumeTarget, prune []resumeTarget)

// resumeGroup derives the picker-group key for a root, mirroring gatherSessions.
func resumeGroup(root string) string

// executeResumeStep performs one target's side effect (impure):
//   recreate → createTmuxSession + setSessionRoot
//   prune    → removeSessionState + clearSessionStateFiles
// Returns the target with Status/Err filled in.
func executeResumeStep(t resumeTarget) resumeTarget
```

`main.go`/`commands.go`:
- Register `resume` in `runCommand`. `resumeCommand(args)` parses an optional single project-name arg via `flag.NewFlagSet("resume", …)`, calls `listSessionStates()` + `planResume`, and hands the plan to a new picker entry point.

`picker.go`:
- `runPickerResuming(recreate, prune []resumeTarget) error` — builds the model with a `*resumeJob` attached, then runs the bubbletea program. The job drives sequential execution via `tea.Cmd`s.

No new dependencies.

## Group matching

`resumeGroup(root)` returns `filepath.Base(workspaceRootFromPath(root))` when the root is under a known worktree dir, else `filepath.Base(root)` — identical to how `gatherSessions` computes `group`. The `<project>` argument is matched case-insensitively against this key. This means `domux resume audrey-app` restores the main `audrey-app` checkout **and** all its `workspace-N` worktrees, because they share the group.

A `<project>` that matches no saved group → the command prints `no saved sessions for "<project>"` and exits non-zero, listing available groups as a hint. (Pure check on the plan; no picker launched.)

Bare `domux resume` with zero saved states → prints `no saved sessions to resume` and exits 0.

## Picker restoring mode

### Model additions

```go
type resumeJob struct {
    recreate []resumeTarget
    prune    []resumeTarget
    idx      int  // next recreate index
    pruneIdx int  // next prune index
    done     bool
    nRecreated, nRunning, nPruned, nFailed int
}
```

`pickerModel` gains one field: `resume *resumeJob` (nil for a normal launch — all existing behavior untouched). New message: `resumeStepMsg{ target resumeTarget }`.

### Flow

1. `runPickerResuming` builds the model, `Init()` notices `m.resume != nil` and batches the normal commands (`pickerRefreshCmd`, spinner, PR refresh) **plus** the first `resumeStepCmd`.
2. `resumeStepCmd` runs `executeResumeStep` for the current target (prune entries first, then recreate entries — order keeps the pruned names out of the soon-to-populate list) and returns a `resumeStepMsg`.
3. On `resumeStepMsg`: increment the matching counter, advance the index, and either dispatch the next `resumeStepCmd` or mark `done`.
4. The banner renders in the logo header status box: while running, `restoring N/total…`; when `done`, a transient summary (rides the existing 5s status TTL): `restored 6 · running 0 · pruned 1`. If any step failed, the summary includes `· N failed` and uses the error style.
5. The existing 2-second `pickerRefreshCmd` independently re-runs `gatherSessions()`, so each recreated session appears in the real grouped list — with its label, todos, recap — within ~2s of coming back. The restore job only narrates; the list fills itself.

### Rendering

Reuses the existing status box in `logoHeaderLines` (top-right of the logo) for the banner — no new view. Pruned/skipped sessions never appear in the live list (their tmux session is gone), so they're surfaced only in the final summary count; the count is the user's signal that something was dropped.

The restore banner takes priority over incidental `m.status` writes while `!m.resume.done`.

### Interaction during restore

The picker is fully interactive while restoring — arrow keys, filter, even `enter` to switch into an already-recreated session all work, because the job advances via background `tea.Cmd`s rather than blocking. Pressing `enter` on a session that hasn't been recreated yet behaves exactly as switching to a missing session does today (tmux errors surface in status); in practice the recreate loop outruns the user.

## Lifecycle

### Plan (pure)

```
planResume(states, filter):
  for each state with non-empty Name+Root:
    group = resumeGroup(state.Root)
    if filter != "" and !equalFold(group, filter): skip entirely
    if !dirExists(state.Root):   prune  += target{Name, Root, Group}
    else:                        recreate += target{Name, Root, Group}
  sort both by Group then Name (stable, matches picker order)
```

### Execute (per target, sequential)

```
prune target:
  removeSessionState(Name)
  clearSessionStateFiles(home, Name)   // legacy .tmux-* files
  status = pruned

recreate target:
  if tmuxSessionExists(Name): status = already running; return
  createTmuxSession(Name, Root)         // starts tmux server if down
  setSessionRoot(Name, Root)            // re-pin Root/TodoPath
  status = recreated
  on any error: status unchanged, Err set, counted as failed
```

`createTmuxSession` failing for one target does not abort the rest — each step is independent; the failure is counted and the loop continues. This mirrors provisioning's "no partial-state rollback; failures are visible and re-runnable" stance.

## Error handling

- Missing root → not an error; it's the prune path.
- `createTmuxSession` error (e.g. name collision with a session tmux already has under a different root) → caught per-target, counted as `failed`, surfaced in the summary. Re-running `domux resume` is safe and idempotent: already-running sessions report `already running` and are left alone.
- `listSessionStates` error → command aborts before launching the picker, prints the error, exits non-zero.
- Unknown `<project>` → friendly message + available groups, exit non-zero. No picker.
- Running `domux resume` from outside tmux is the normal case (right after a reboot). `createTmuxSession` starts the server; the picker then attaches like any first launch.

## Testing

`resume_test.go` (new) — the pure planner needs no tmux:

- `TestPlanResume_PartitionsMissingRoots` — states with existing vs. missing roots split into recreate vs. prune.
- `TestPlanResume_FilterByGroup` — filter selects only the matching group; non-matching states are neither recreated nor pruned.
- `TestPlanResume_WorkspaceSharesMainGroup` — a `workspace-N` root and its main checkout land in the same group; `filter=<project>` picks up both.
- `TestPlanResume_FilterCaseInsensitive` — `AUDREY-APP` matches group `audrey-app`.
- `TestPlanResume_SkipsBlankNameOrRoot` — states missing Name or Root are ignored.
- `TestResumeGroup_StripsWorkspaceSuffix` — root under `.domux/worktrees/workspace-2` → group is the main checkout's base name.

`resume_test.go` execution tests (tmux shimmed via `PATH`, mirroring `commands_test.go`):

- `TestExecuteResumeStep_RecreatesMissingSession` — session absent → `createTmuxSession` invoked, status `recreated`, state root re-pinned.
- `TestExecuteResumeStep_SkipsExistingSession` — `tmuxSessionExists` true → status `already running`, no `new-session` call.
- `TestExecuteResumeStep_PruneRemovesState` — prune target → state json gone, legacy `.tmux-*` files gone, status `pruned`.

`commands_test.go` additions:

- `TestResumeCommand_UnknownProject` — non-matching arg → error mentioning the project, no picker.
- `TestResumeCommand_NoSavedSessions` — empty states dir → friendly exit, no error.

`picker_test.go` additions:

- `TestResumeJobAdvancesSequentially` — feed a model a `resumeJob` with two targets; simulate two `resumeStepMsg` round-trips; assert counters and `done`.
- `TestResumeBannerRendersProgress` — model mid-restore renders `restoring 1/2…` in the header; when done renders the summary string.

## Out of scope (parked for later)

- Restoring window/pane layout or split configuration per session.
- Reviving the server session's long-running process (was offered, declined — bare shells only).
- A `--dry-run` that prints the plan without executing. The summary count already reports what happened; add later if wanted.
- Auto-resume on shell startup / login hook. This stays an explicit user command.
