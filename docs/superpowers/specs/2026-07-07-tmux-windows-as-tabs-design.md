# Tmux windows as tabs — multiplying sessions without a git root

**Date:** 2026-07-07
**Status:** Approved, ready for implementation plan

## Problem

The switcher has exactly one way to multiply a session: **git worktrees**. Under a
git-root group you press `p` and `provisionWorkspace` spins up a `workspace-N`
worktree as its own tmux *session*. But a session whose current directory is not
inside a git repo has `Root == ""`, so `p` dead-ends with
`"no git root for this row"` — there is no worktree to create.

Meanwhile the user already runs multiple *tmux windows (tabs)* inside a single
session by hand (e.g. a `domux` session with tabs `1 · prod uk` and
`2 · merge queue`). Those windows are invisible to the switcher beyond a bare
count, and there is no way to jump straight to a specific one.

## Idea

Give every session a second multiplication mechanism — **tmux windows** — and make
windows first-class, jumpable rows in the switcher.

Unified mental model:

- **Git repo → parallelize with worktrees** (`p`, unchanged).
- **Any session → parallelize with windows/tabs** (`w`, new).

The switcher surfaces both. This retires the "no git root" dead-end *for the grow
action*: no-git-root sessions grow with `w` instead of `p`.

## Decisions (from brainstorming)

- **Scope:** any session with **>1 window** lists its windows as jumpable sub-rows
  (git-root or not). Single-window sessions render exactly as today.
- **Grow keys are separate verbs:** `w` adds a window to **any** session; `p` stays
  worktree-only for git-root rows. Windows and worktrees are independent tools.
- **Layout:** the session stays as one primary row; windows hang beneath it as
  sub-rows (like tasks today), above the session's tasks.
- **Per-window status/recap:** each window row carries its own inline AI badge; its
  recap (`※` line) hangs under that window and is shown only when details are on
  (the existing `tab` / `m.showDetails` toggle). Default view stays tight.
- **New window naming:** `w` prompts for a name inline (reusing the label input
  overlay); the window is named from birth.
- **Window deletion:** out of scope for v1 — close tabs with native tmux. Create +
  navigate only.

## Data model

### Enumerate windows in `gatherSessions`

`gatherSessions` already runs `tmux list-windows -t <sess>` purely to count windows
(`info.Windows`). Change the format string to enumerate:

```
tmux list-windows -t <sess> -F "#{window_index}\t#{window_name}\t#{window_active}\t#{pane_current_path}"
```

Populate a new slice on `sessionInfo`:

```go
type windowInfo struct {
    Index       int
    Name        string
    Active      bool
    Path        string // window's active-pane cwd
    Claude      string
    Codex       string
    ClaudeLabel string
    CodexLabel  string
    Recap       string
}
```

`sessionInfo` gains `Windows []windowInfo`. When `len(Windows) <= 1` the slice is
left empty and nothing renders differently from today. `info.Windows int` (the
count) may remain or be derived from `len` — implementer's call, keep whichever
minimizes churn.

### Per-window AI status — data already exists

`state.AI` is keyed `agent:window_pane` (e.g. `claude:2_0`). Today
`aggregateAIStatesFromSession` collapses **all** panes into one session-level
`AIStates`. Add a sibling:

```go
func aggregateAIStatesByWindow(state *SessionState) map[int]AIStates
```

that buckets by the `window` half of the key instead of collapsing. Reuse
`normalizeAIState` (single source of truth — untouched) and the same WAITING-wins
merge semantics per window. The existing session-level aggregate stays as the
fallback for single-window sessions.

### Per-window recap

Recap resolves by **cwd** via `bestLiveSession(liveClaude, paths...)`. For a window,
match on that window's `Path` (its active-pane `pane_current_path`).

**Known limitation (accepted for v1):** when two windows share a cwd and both run
Claude, cwd alone cannot disambiguate them — resolution falls back to
most-recently-updated. This is exactly today's session-level behavior, now scoped to
the window. PID-precise matching is feasible later (`claudeSession.Pid` exists) but
is deliberately out of v1 scope.

> Note: moving recap down onto window rows is *more* correct than today's
> session-level recap, which already picks an arbitrary window's recap when several
> run Claude.

## Display

### New row kind

Add `rowWindow` alongside `rowSession` / `rowTask`, carrying a `*windowInfo` and the
parent session name. In the row-flattening step (where tasks are appended under
their session), when `len(session.Windows) > 1`, append one `rowWindow` per window —
directly under the session row, above that session's task rows, in window-index
order.

Ordering under a session:

```
◇ domux on main              ← session row (Enter jumps to active window)
    ▸ 1 · prod uk            ← window rows, window-index order
    ▸ 2 · merge queue     ⠙ Hullaballooing
    ※ …recap (tab-gated)…    ← per-window recap hangs under its window
    ○ task one               ← tasks still below
```

### `renderWindow` (sibling to `renderTask`)

- Indent 8 to align with tasks / recap (keeps the existing column grid).
- Glyph `▸` in a dim style; the **active** window's `▸` gets teal/bold so the
  current tab is visible.
- `{index} · {name}` — mirrors the real tmux tab form.
- Inline AI badge via the **existing** `renderAIBadges(win.Claude, win.Codex,
  win.ClaudeLabel, win.CodexLabel, m.spinnerFrame)` — no new badge code.
- Recap: reuse the exact `m.showDetails`-gated `※` block from `renderSession`, fed
  the window's recap, hung under the window row.

### Cursor & selection

`▸` / cursor prefix columns already exist for task rows; window rows reuse them.
`selectedSession()` returns nil on non-session rows today — add a parallel path so
the cursor can resolve a window row (both to its `windowInfo` and its parent
session, e.g. for `w`).

## Navigation & actions

### Enter on a window row → jump to that window

Today `selectRow` sets `m.selected = session.Name`; the deferred attach (run after
bubbletea releases the tty — see `runPickerProgram`) calls
`attachTmuxSession(name)`. Widen the carried value to an optional
`session:window_index` target:

- `selectRow` on a `rowWindow` sets `m.selected = "domux:2"`.
- `attachTmuxSession` already shells `switch-client -t <name>` /
  `attach-session -t <name>`; both accept a `session:window` target verbatim, so
  this is a one-line widening of the passed string — no new attach machinery.
- Enter on the **session** row is unchanged (lands on the active window).

### `w` — new window on any session

Bind `w` on `rowSession` (and `rowWindow`, resolving to its parent session), git-root
or not — no "no git root" gate. Reuse the existing inline label-input overlay
(`labelEditing` / `labelInput` pattern): press `w`, type the name, Enter. Then issue,
via a `pickerActionMsg{Action: "window"}` round-tripped through `applyPickerAction`
(matching `provision` / `label`):

```
tmux new-window -t <sess> -n <name> -c <cwd>    # cwd = session's current path
```

Empty name cancels. On success the 2s refresh picks up the new window; set the
status line and move the cursor to the new window row.

### `p` — unchanged

Still worktree-provision, still git-root-only. On a no-git-root row keep the guard,
but update the message to point the way:

```
no git root — press w to add a window
```

### Help overlay

Add a `w  new window` line to `renderHelpOverlay`.

## Edge cases

- **1 window** → `Windows` slice empty, zero new rows, byte-identical to today.
- **cwd changes** (user `cd`s in a pane) → window `pane_current_path` re-read each 2s
  refresh; recap/status follow it.
- **Window with no Claude** → no badge, no recap — just the `▸ index · name` jump
  target.
- **Shared-cwd ambiguity** → documented fallback to most-recently-updated (same as
  today's session-level behavior).
- **Cursor stability across refresh** → refresh currently re-homes the cursor by
  session name; extend the same restore to remember a window target so the 2s tick
  doesn't bounce the cursor off a window row.

## Non-impact / legacy

Touches only: picker gather + render, the attach target string, and one new
`new-window` action. Untouched: the `~/.tmux-*` legacy bridge, `SessionState` JSON
schema, TODO files, install commands. `state.AI`'s `agent:window_pane` keying
already exists — we read it a new way, not change the format.

## Testing (table-driven, `picker_test.go` style)

- `aggregateAIStatesByWindow` buckets `claude:1_0` / `claude:2_0` into separate
  windows; WAITING-wins preserved per window.
- Row flattening: 1 window → no window rows; 3 windows → 3 `rowWindow` in index
  order, under the session, above tasks.
- `renderWindow`: active-window glyph styling; badge present/absent; recap gated by
  `showDetails`.
- Enter on window row → `m.selected == "sess:2"`; Enter on session row →
  `m.selected == "sess"` (unchanged).
- `w` opens the input; submit issues `new-window -n <name> -c <cwd>`; empty name
  cancels.
- `p` on a no-git-root row → updated guidance message.
