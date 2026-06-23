# domux-communicate — inter-worktree agent messaging

**Date:** 2026-06-23
**Status:** Approved design

## Problem

domux runs each worktree's Claude agent in its own tmux pane (e.g.
`workspace-2:1.0`). Today an agent in one worktree has no first-class way to
hand work to the agent in another worktree. We proved by hand that tmux
`send-keys` / `capture-pane` against a peer's pane works — the canonical case is
a handoff: *"I built X on branch Y, here's the doc, please pull it in."* This
design productizes that mechanism so any agent can address a peer **by worktree
name**, send it a message, and read its output, without scanning panes.

## Goals

- Address a peer worktree **by name**, reusing the session→worktree mapping
  domux already maintains. Callers never scan panes themselves.
- Three CLI verbs: `domux send`, `domux read`, `domux peek`.
- A Claude Code plugin skill (`/domux-communicate`) so any agent in any worktree
  can invoke this naturally.
- Robustness: safe quoting for arbitrary message text, a capture-first idle
  check so we don't interrupt a peer mid-generation, and an attribution prefix
  so the peer knows the message came from a peer agent — not the human.

## Non-goals (YAGNI)

No message daemon or inbox, no broadcast/fan-out, no reply threading, no new
on-disk state. Just three verbs over the mapping that already exists.

## Architecture

One new Go file `communicate.go` holding the subcommand handlers plus a set of
**pure helper functions** (the test surface). Subcommands register in
`runCommand` (`commands.go`). A standalone plugin lives at
`plugins/domux-communicate/`, registered in `.claude-plugin/marketplace.json`.

Two load-bearing facts, confirmed against live state:

- A Claude agent pane reports the Claude version as its `pane_current_command`
  (e.g. `2.1.186`); shells report `zsh`/`bash`. So a Claude pane is detectable
  by a version-shaped command (regex `^\d+\.\d+`).
- domux's `SessionState.AI` map already records where Claude runs:
  `"claude:1_0": "CLAUDING"` ⇒ the agent is at tmux pane `1.0` of that session.
  The pane key encodes `#{window_index}_#{pane_index}`, so `1_0` → `1.0`.

All tmux interop is `exec.Command` only (no shell). Because message text is
passed as a single argv element to `tmux send-keys -t T -l <msg>`, backticks,
quotes, `;`, `$`, and paths are safe with no escaping.

## Name → pane resolution (the spine)

`resolveCommTarget(name, paneFlag) (commTarget, error)` where

```go
type commTarget struct {
    Name    string // the name the caller passed
    Session string // resolved tmux session
    Root    string // worktree root (may be "")
    Label   string // human label if any
    Pane    string // "1.0"
    Target  string // "workspace-2:1.0"
    Command string // pane_current_command at resolve time (for safety checks)
}
```

1. **Name → session.** Pure `matchSessionsByName(name, states)` over
   `listSessionStates()`, matching case-insensitively against `state.Name`,
   `filepath.Base(state.Root)` (worktree dir / branch), or `state.Label`.
   - 0 matches → fall back to a live tmux session whose name equals `name`
     exactly (`tmuxSessionExists`); synthesize a target with empty state.
   - 0 still → error: `no worktree matches "<name>" (try: domux peek)`.
   - >1 matches → ambiguous error listing the candidates.
2. **Session → Claude pane.**
   - `--pane W.P` set → use it (after `normalizePaneSpec`, which accepts `1_0`
     or `1.0`).
   - else `claudePaneSpecsFromState(state)` returns `"w.p"` for each
     `claude:*` AI key. Exactly 1 → use it.
   - 0 from state → live scan: `tmux list-panes -t <session>` and keep panes
     whose `pane_current_command` satisfies `looksLikeClaudeCommand`. 1 → use;
     0 and the session has exactly one pane total → use that pane; otherwise
     error.
   - >1 candidates with no `--pane` → error listing them, suggest `--pane W.P`.

`matchSessionsByName`, `claudePaneSpecsFromState`, `paneKeyToTarget`,
`normalizePaneSpec`, and `looksLikeClaudeCommand` are pure and unit-tested. The
live-tmux fallback is isolated behind a small seam so the state-only path is
testable without a running tmux server.

## Subcommands

### `domux send <name> <message…>`

Flags: `--from NAME`, `--no-enter`, `--pane W.P`, `--wait[=DUR]`.

1. Resolve the target.
2. Self-send guard: if the target pane is the caller's own pane, refuse.
3. Compose the attributed message:

   ```
   [domux peer message — from worktree "workspace-3" (a Claude agent, not your operator)]

   <message>
   ```

   `--from` defaults to the **sender's** session label-or-name (resolved from
   `currentTmuxSession`); outside tmux it defaults to `a peer agent`.
4. Idle handling via `isPaneBusy(target)`:
   - AI state for the pane is `CLAUDING` ⇒ busy; `WAITING`/`IDLE` ⇒ idle.
   - state unknown ⇒ **capture-first diff**: capture the tail, sleep ~600ms,
     capture again; changed ⇒ busy.
   - **Default (no `--wait`)**: send immediately. Claude Code queues input, so
     nothing is lost; if the peer was busy, the confirmation says
     `peer was generating — message queued`.
   - **`--wait[=DUR]`**: poll (~750ms) until idle, up to `DUR` (default 60s),
     then send; on timeout, error without sending.
5. Send: `tmux send-keys -t T -l <full>`, then a **separate**
   `tmux send-keys -t T Enter`. `--no-enter` skips the Enter, staging the text
   in the peer's input box for review.
6. Print a confirmation: target, resolved name, `--from`, and queued/sent state.

### `domux read <name>`

Flags: `--lines N` (default 50), `--pane W.P`.

Resolve the target, then `tmux capture-pane -t T -p -S -<N>` and print verbatim.

### `domux peek`

No args. `tmux list-panes -a -F "<fmt>"`, keep panes where
`looksLikeClaudeCommand(pane_current_command)`, and cross-reference session
state. Print a table: addressable **name** (session name), `session:pane`, AI
**state** (CLAUDING / WAITING / idle / unknown), and **task** (`pane_title`,
falling back to the focused/top todo). This is the discovery path that tells a
caller which names `send`/`read` accept.

## Plugin

`plugins/domux-communicate/`:

- `.claude-plugin/plugin.json` — name `domux-communicate`, version `0.1.0`,
  description, author, homepage. No dependencies (it only shells out to the
  `domux` binary already on PATH).
- `skills/domux-communicate/SKILL.md` — `/domux-communicate`, styled like the
  existing `codex-review` skill: frontmatter (`name`, `description`) then a body
  that teaches the peer-handoff workflow — `peek` to discover, `send` to hand
  off, `read` to check the reply — and spells out the attribution prefix and
  idle/`--wait` semantics.

Registered in `.claude-plugin/marketplace.json` alongside `implement-pipeline`.

## Error handling

Actionable messages: unknown name (suggest `domux peek`), ambiguous name (list
candidates), no Claude pane found, multiple panes (suggest `--pane`), `--wait`
timeout, self-send guard. tmux failures surface `exec.Command` combined output,
matching existing helpers like `createTmuxSession`.

## Testing

`communicate_test.go`, Go `testing`, table tests in the repo style
(`TestBehaviorCondition`). Pure-function coverage:

- `matchSessionsByName` — match by session name, by root basename, by label;
  case-insensitivity; ambiguity returns >1.
- `claudePaneSpecsFromState` / `paneKeyToTarget` — AI map → pane specs/targets;
  ignores non-claude and malformed keys.
- `normalizePaneSpec` — `1_0`/`1.0` → `1.0`; rejects junk.
- `looksLikeClaudeCommand` — `2.1.186` true, `zsh`/`bash`/`""` false,
  `claude` true.
- `attributionPrefix` / `formatMessage` — header text and body assembly,
  including the default `--from`.
- arg parsing for `send` / `read` / `peek` (flag combinations, missing args).

A `manifest_test.go`-style assertion validates the new `plugin.json` and that
`marketplace.json` lists `domux-communicate`.

## Docs

README section documenting `send` / `read` / `peek` and the plugin, using the
workspace-3 → workspace-2 handoff as the worked example.
