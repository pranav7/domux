# Resume window layout + agent sessions

**Date:** 2026-07-13  
**Status:** Spec — ready for implementation

## Problem

After `domux resume` recreates sessions, each session is a single empty shell at the saved root. The user must manually:
- Recreate any tmux windows (tabs) they had before the restart
- Restart Claude, Codex, or OpenCode in each window
- Navigate to the right prior conversation via `claude --resume <id>`, `codex resume <id>`, or `opencode --session <id>` — IDs they don't have memorized

This loses most of the workspace shape. Sessions come back, but the fine-grained window layout and agent conversations are manual recovery work.

Meanwhile, the picker already renders windows as first-class rows when `len(Windows) > 1` (per the 2026-07-07 tmux-windows-as-tabs spec), but that rendering has visual inconsistencies: AI badges + recaps hop between session and window rows depending on window count, and the `◇` main-worktree diamond is redundant now that every session shows its windows.

## Goal

1. **Resume window layout:** `domux resume` recreates each session's window list (name + cwd per window) and launches the correct agent CLI in each window with `--resume <id>` / `--session <id>` flags, restoring the exact prior conversations.
2. **Consistent picker display:** Always expand windows (even single-window sessions), show agent icons next to window index numbers, and remove the redundant `◇` diamond.

## Non-goals

- Restoring pane splits within a window — windows only, not panes.
- Persisting which agent was in which window at snapshot time — we resolve that **live** at resume time by cwd-matching against each agent's own session registry.
- Supporting agents beyond Claude/Codex/OpenCode — extensible later but not v1.

## Supersedes / extends

- **2026-07-07 tmux-windows-as-tabs:** That spec introduced window rows for multi-window sessions. This spec makes them **always present** (even single-window) and adds resume behavior.
- **2026-06-29 resume-sessions:** That spec created bare-shell resume. This extends it to recreate windows + relaunch agents per window.

## Data model

### Persist window list in `SessionState`

Add to `session.go`:

```go
type SessionState struct {
    // ... existing fields ...
    Windows []WindowSnapshot `json:"windows,omitempty"`
}

type WindowSnapshot struct {
    Index int    `json:"index"`
    Name  string `json:"name"`
    Cwd   string `json:"cwd,omitempty"`
}
```

**When to snapshot:** Piggyback on existing `saveSessionState` call sites (ai-state changes, label/server writes already trigger a save). On each save, if we're inside tmux and can identify the current session, snapshot the window list via `tmux list-windows -F "#{window_index}\t#{window_name}\t#{pane_current_path}"` and replace `state.Windows` wholesale. This keeps the snapshot fresh without adding new periodic timers.

**Agent field omitted:** We deliberately do **not** persist `Agent` in `WindowSnapshot`. The agent-to-resume is resolved **live at resume time** by scanning each agent's session registry for the most-recently-updated entry whose cwd matches the window's saved path. This avoids stale agent references when a user switches agents between snapshots, and mirrors the existing recap resolution pattern.

### Resume-time agent session registries

Extend the existing `recap.go` pattern (which reads `~/.claude/sessions/*.json` and matches by cwd for recaps) to all three agents. Add to `recap.go`:

```go
type agentSession struct {
    Agent     string // "claude" | "codex" | "opencode"
    SessionID string
    Cwd       string
    UpdatedAt int64  // unix millis
}

// readAgentSessions reads all three agent session registries and returns
// a unified list ordered by UpdatedAt descending (most recent first).
func readAgentSessions() []agentSession

// bestAgentSession returns the most recently updated session whose cwd
// matches one of the given paths, and true; (zero, false) when none match.
func bestAgentSession(sessions []agentSession, paths ...string) (agentSession, bool)
```

**Claude:** `~/.claude/sessions/<pid>.json` has `sessionId` + `cwd` + `updatedAt` (a real field in the JSON, milliseconds since epoch — not derived from mtime).

**Critical: do not reuse `readClaudeSessions()` as-is.** It filters entries through `pidAlive(rec.Pid)`, which is correct for the picker's live recap (only show a recap for a session that's actually running right now) but **wrong for resume** — the entire point of `domux resume` is recovering from a reboot, at which point every prior pid is by definition dead, and `pidAlive` would zero out every candidate. Add a second reader, e.g. `readClaudeSessionsForResume()` (or refactor `readClaudeSessions` to take an `includeDead bool` / skip the alive filter), that reads the same files without the liveness gate. Map each to `agentSession{Agent: "claude", SessionID: sessionId, Cwd: cwd, UpdatedAt: updatedAt}`.

**Codex:** Parse `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` — each file's first line is `session_meta`. `payload.id` always matches the UUID in the filename (use this field — `payload.session_id` is the *parent* thread ID for subagent-spawned rollouts and diverges from the filename in that case). **Skip rollouts where `payload.thread_source == "subagent"`** — those are sub-thread transcripts spawned by review/subagent work, not resumable top-level sessions; the parent session already has its own separate rollout file which gets picked up normally. Use `payload.id` and `payload.cwd`. No `updatedAt` field exists in the payload, so use the file's mtime (`os.Stat`) as the last-updated signal. Map to `agentSession{Agent: "codex", SessionID: payload.id, Cwd: payload.cwd, UpdatedAt: mtimeMillis}`.

**OpenCode:** Shell out to `opencode session list --format json`, which returns `[{id, directory, updated}, ...]` (updated is unix millis). Map to `agentSession{Agent: "opencode", SessionID: id, Cwd: directory, UpdatedAt: updated}`.

**Matching semantics:** For a window with saved cwd `/Users/x/projects/audrey/workspace-1`, find the most-recently-updated session whose `Cwd` equals that path, across all three agents. If multiple agents have matching sessions (e.g. both Claude and Codex ran in that dir), pick the most recent by `UpdatedAt`. If no match, leave the window with an empty shell.

**Cwd equivalence:** Exact string match only — no symlink resolution, no parent-directory fallback. This is the same simplicity chosen for recap matching.

## Resume behavior changes

### Extend `executeResumeStep` in `resume.go`

After `createTmuxSession(name, root)` + `setSessionRoot(name, root)`, if `state.Windows` is non-empty:

1. **Recreate window shells:**
   - Window index 1 is the default window created by `tmux new-session`. Rename it: `tmux rename-window -t <session>:1 <state.Windows[0].Name>`.
   - For windows 2+, issue `tmux new-window -t <session> -n <name> -c <cwd>` per `WindowSnapshot`.

2. **Resolve agent sessions:**
   - Call `sessions := readAgentSessions()` once (shared across all windows in this session).
   - For each window, call `bestAgentSession(sessions, window.Cwd)`. If found, the returned `agentSession` has the agent type + session ID.

3. **Launch agents:**
   - For each window with a matched agent session, send the resume command into that window's shell via `tmux send-keys -t <session>:<window.Index> '<command>' C-m`. Commands:
     - Claude: `claude --resume <sessionID>`
     - Codex: `codex resume <sessionID>`
     - OpenCode: `opencode --session <sessionID>`
   - If no match, leave an empty shell (graceful fallback).

**Error handling:** If any window creation fails (unlikely — tmux new-window is robust), log the error and continue with the next window. The resume job already counts per-session failures; window-level failures are absorbed and logged but don't fail the session.

**Performance:** The agent registry scan (`readAgentSessions`) runs once per session being resumed, not per window. For 10 sessions with 3 windows each = 10 scans, not 30.

### Resume command summary update

The existing `resumeJob.summary()` line (`restored N · running M · pruned P`) gains detail when windows are involved. Extend the summary format:

```
restored 6 (18 windows, 15 agents resumed) · running 0 · pruned 1
```

Count total windows recreated and total agent sessions launched across all recreated sessions. This gives the user a signal that the deeper restore happened.

## Picker display changes

### Always expand windows

**Change:** `rowsFromEntries` in `picker.go` always emits `rowWindow` rows per window, even for single-window sessions. Drop the `len(session.Windows) > 1` gate entirely.

**Rationale:** Consistent layout — every session is rendered the same way regardless of window count. Removes the visual hop where AI badges/recaps jump from session row (1 window) to window rows (2+ windows).

**Impact:** Single-window sessions gain one indented window row. The session row loses its inline AI badge + recap (now exclusively on the window row). This is a **visual breaking change** but increases consistency.

### Agent icon glyphs next to window index

**Change:** `renderWindow` in `picker.go` replaces the current `{index} · {name}` separator with an agent icon glyph, showing which agent is in that window:

```
  1 ✳ main          ✦ Boogieing   ← Claude (orange ✳ #DE7356)
  2 C delete-firm                 ← Codex (blue C)
  3 O scratch                     ← OpenCode (pink O)
  4   empty                       ← no agent (blank space)
```

**Glyph mapping:**
- Claude → `✳` (U+2733 Eight Spoked Asterisk) in `#DE7356` (Claude brand orange)
- Codex → `C` in blue (`#89DCEB` from Catppuccin Mocha sapphire)
- OpenCode → `O` in pink (`#F5C2E7` from Catppuccin Mocha pink)
- No agent → space (keeps alignment)

**Source of agent info:** The existing `windowInfo.Claude/Codex/OpenCode` fields, already populated from `state.AI` by `aggregateAIStatesByWindow`. This data exists live today; we're just rendering it as a glyph instead of only in the badge.

**Style definitions:** Add three new styles to the `picker.go` styles block:

```go
var (
    // ... existing styles ...
    pAgentClaude = lipgloss.NewStyle().Foreground(lipgloss.Color("#DE7356"))
    pAgentCodex  = lipgloss.NewStyle().Foreground(sapphire)
    pAgentOpenCode = lipgloss.NewStyle().Foreground(pink)
)
```

### Remove `◇` main-worktree diamond

**Change:** Delete the `mainGlyph` logic in `renderSession` — no longer render `◇` or `◌` on the session line.

**Rationale:** Now that every session **always** shows its windows, the main-worktree marker is redundant. The user can already distinguish main checkouts from workspace-N worktrees by session name (e.g. `audrey-app` vs `workspace-1`), and the diamond adds visual noise without information.

**Impact:** Session rows lose the 5-column prefix `[waiting bar] [cursor] [diamond] ` and collapse to `[waiting bar] [cursor] ` (4 columns). This is a **visual breaking change** but simplifies the display and removes an arbitrary glyph.

### Updated rendering example

Before (multi-window session):
```
AUDREY-APP ──────────────────
 ◇ audrey-app on main | Impersonation + Home Tenant  ✦ Boogieing
   › 1 · main
     2 · delete firm
```

After (always expanded, agent icons):
```
AUDREY-APP ──────────────────
  audrey-app on main | Impersonation + Home Tenant
   › 1 ✳ main          ✦ Boogieing
     ※ Building an admin force-delete for a firm…
     2   delete firm
```

Single-window session before:
```
DOMUX ───────────────────────
 ◇ domux on main  ✦ Flummoxing
   ※ pin per-window recap to the claude session…
```

Single-window session after:
```
DOMUX ───────────────────────
  domux on main
   › 1 ✳ main          ✦ Flummoxing
     ※ pin per-window recap to the claude session…
```

## Test changes

### Flip existing tests (`picker_test.go`)

- **`TestRowsFromEntriesSingleWindowNoWindowRows`** — rename to `TestRowsFromEntriesSingleWindowEmitsWindowRow` and invert the assertion: single-window sessions **do** emit one `rowWindow`.
- **`TestRenderSessionMarksMainWorktree`** — delete entirely (the diamond is gone).
- **`TestRenderSessionBadgeSuppressedForMultiWindow`** / **`ShownForSingleWindow`** — collapse into one test: AI badges are **always suppressed** on the session line (they live on window rows now).
- **`TestRenderSessionRecapSuppressedForMultiWindow`** / **`ShownForSingleWindow`** — same collapse: recaps are **always suppressed** on the session line.

### New tests

**`picker_test.go`:**
- `TestRenderWindowAgentGlyph` — window with `Claude: "CLAUDING"` renders `✳` in orange; `Codex: "CODEXING"` renders `C` in blue; `OpenCode: "CODING"` renders `O` in pink; empty window renders space.
- `TestRenderWindowGlyphReplacesMiddleDot` — confirm `·` is gone; glyph is adjacent to window name.

**`session_test.go`:**
- `TestSaveSessionStateSnapshotsWindows` — save a session while inside tmux with 3 windows; verify `state.Windows` has 3 entries with correct `Index/Name/Cwd`.
- `TestSaveSessionStateEmptyWindowsWhenOutsideTmux` — save when `$TMUX` is unset; verify `state.Windows` is empty (no-op when not in tmux).

**`recap_test.go`:**
- `TestReadAgentSessions` — mock all three registries (Claude files, Codex rollouts, OpenCode JSON stdout); verify unified `[]agentSession` sorted by `UpdatedAt`.
- `TestBestAgentSession` — given sessions from multiple agents with overlapping cwds, confirm most-recent wins; given no match, confirm `(zero, false)`.

**`resume_test.go`:**
- `TestExecuteResumeStepRecreatesWindows` — session with 3 `WindowSnapshot` entries recreates 3 tmux windows with correct names/cwds.
- `TestExecuteResumeStepLaunchesAgents` — window with cwd matching a Claude session sends `claude --resume <id>` via `tmux send-keys`.
- `TestExecuteResumeStepFallsBackToEmptyShell` — window with no matching agent session leaves an empty shell (no send-keys call).
- `TestResumeJobSummaryCountsWindowsAndAgents` — job with 2 sessions, 5 total windows, 4 agent matches renders `restored 2 (5 windows, 4 agents resumed)`.

## Edge cases

- **Window with changing cwd:** If the user `cd`s in a pane, the next `saveSessionState` snapshots the new cwd. Resume launches the agent at that new cwd. The cwd in `WindowSnapshot` is whatever was active when the last save happened, not the original cwd.
- **Multiple agents in one cwd:** If both Claude and Codex ran in the same directory and both have live sessions, `bestAgentSession` picks whichever was updated most recently (the session registry is sorted by `UpdatedAt` descending, and we take the first match). This is a **best-effort** heuristic; true disambiguation would require tracking which agent was in which pane at snapshot time (deliberately out of scope).
- **Agent CLI not on PATH:** If `claude`/`codex`/`opencode` is not executable when resume runs, the `tmux send-keys` call succeeds but the shell prints `command not found`. This is visible to the user in that window — a clear signal. Resume does not pre-validate agent availability (same as today's manual `claude --resume` workflow).
- **Session deleted from agent registry:** If the user explicitly deleted a session via `claude session delete <id>` / `codex delete <id>` / `opencode session delete <id>` between shutdown and resume, the registry match fails and that window gets an empty shell. The snapshot doesn't pin the agent session existence; it's a best-effort cwd-based lookup.
- **Snapshot frequency:** Windows are snapshotted on every `saveSessionState` call, which happens whenever ai-state / label / server changes. This is frequent enough in active use that the snapshot stays reasonably fresh (within minutes of the last activity). A completely idle session (no ai-state writes, no label changes) won't snapshot until the user does something that triggers a save. Acceptable — idle sessions have no new window layout to capture.

## Non-impact / legacy

Touches only: `session.go` (`SessionState.Windows` + snapshot logic), `recap.go` (agent session readers), `resume.go` (window recreation + agent launch), `picker.go` (display rendering). Untouched: `~/.tmux-*` legacy bridge (still works), TODO files, install commands, `state.AI` keying (already `agent:window_pane`, unchanged).

## Implementation notes

- **Agent session readers share a pattern:** Each of the three `read*Sessions` functions in `recap.go` returns `[]agentSession` with `Agent` / `SessionID` / `Cwd` / `UpdatedAt` populated. `readAgentSessions` concatenates the three and sorts by `UpdatedAt` descending (most recent first). This makes `bestAgentSession` a simple linear scan — take the first whose cwd matches.
- **Snapshot is opt-in per save:** The `saveSessionState` call sites are scattered (`state_commands.go`, `commands.go`, `picker.go`). The snapshot logic should be a helper `snapshotWindowsIfInTmux(session) []WindowSnapshot` called inside `saveSessionState` itself, so all call sites automatically get the snapshot without individual changes.
- **Resume's send-keys timing:** After `createTmuxSession` + `setSessionRoot` + all `tmux new-window` calls, the windows exist but the shells may still be initializing (sourcing `.zshrc`, etc.). The `tmux send-keys` calls should happen **after** a small settle delay (e.g. 500ms) to avoid sending `claude --resume` into a shell mid-startup. Alternatively, send-keys with a shell guard: `command -v claude >/dev/null && claude --resume <id>` so the command no-ops if the shell isn't ready. The guard approach is safer and avoids arbitrary sleep.
