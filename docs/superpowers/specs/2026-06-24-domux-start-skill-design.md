# Design: `domux-start` plugin skill (+ retire `/start-task`, add `/cr` alias)

Date: 2026-06-24

## Problem

The repo ships a `/start-task` slash command (`assets/claude/commands/start-task.md`)
that `domux install claude` embeds into the binary and writes to
`~/.claude/commands/start-task.md`. It walks an agent through setting up a
tmux+domux workbench: resolve the worktree, branch off fresh `main`, label the
session, then begin the task.

We want that workflow distributed as a proper **marketplace plugin skill**
(`/domux-start`), the way `domux-communicate` is — so an agent auto-invokes it
when kicking off a task. The marketplace skill becomes the single source of
truth, and the embedded `/start-task` command is retired.

Separately: add a short, non-colliding `/cr` alias for the existing
`/codex-review` skill.

## Goals

1. New standalone `domux-start` plugin in the marketplace, carrying the
   task-kickoff workflow as a skill.
2. Retire the embedded `/start-task` command (remove embed, install write,
   tests, README mention).
3. Add a `/cr` command alias that forwards to the `codex-review` skill, without
   touching the built-in `/review`.

## Non-goals

- No change to the `domux start` Go subcommand (`commands.go:114`).
- No change to `domux-communicate` or `implement-pipeline:implement`.
- No re-org of the existing plugins; `domux-start` is additive.

---

## Part A — New `domux-start` plugin

### Layout

```
plugins/domux-start/
  .claude-plugin/plugin.json
  skills/domux-start/SKILL.md
```

### `plugin.json` (mirror `domux-communicate`'s shape)

```json
{
  "name": "domux-start",
  "version": "0.1.0",
  "description": "Kick off a new task in a tmux+domux worktree: resolve the workspace, branch off fresh main, label the session, then start coding.",
  "author": { "name": "pranav7" },
  "homepage": "https://github.com/pranav7/domux"
}
```

### `SKILL.md`

Frontmatter:

```yaml
---
name: domux-start
description: Use when starting/kicking off a new task in a tmux+domux workbench — sets up the workspace before any code: resolves the git worktree root, branches off fresh origin/main, labels the domux session so the human can find it in the switcher, then begins the task. Invoke at the start of task-shaped requests in a domux/tmux session.
---
```

Body: the existing `start-task.md` content, adapted from a command into a skill:

- Keep all five setup steps (resolve workspace → fetch main → branch off fresh
  main → `domux label set` → acknowledge + begin) and the "What domux is" and
  "domux quick reference" sections verbatim where they still apply.
- **Adapt the `$ARGUMENTS` placeholder.** `$ARGUMENTS` is a *command* feature and
  is not substituted in skills. Replace each `$ARGUMENTS` use with prose that
  reads the task from context, e.g. "the task you were asked to start (the skill
  argument, or the request you're acting on)". The skill must work both when
  invoked as `/domux-start <task>` and when auto-triggered mid-conversation.
- Keep the restricted-branches warning (`main`, `master`, `workspace-*`).

### Register in the marketplace

Add a third entry to `.claude-plugin/marketplace.json` `plugins`:

```json
{
  "name": "domux-start",
  "source": "./plugins/domux-start",
  "description": "Kick off a task in a tmux+domux worktree: branch off fresh main, label the session, then start — the /domux-start skill."
}
```

---

## Part B — Retire `/start-task`

### `install.go`

- Remove the `//go:embed assets/claude/commands/start-task.md` directive and the
  `claudeStartTaskCommand` var (lines 17–18).
- Remove the `claudeStartTaskFile` const (line 20).
- In `installClaude`: remove the `cmdPath` line (112), the preview line that
  prints "Would write … the /start-task slash command" (117), and the
  `writeClaudeCommand(cmdPath, …)` call (136).
- Remove the now-unused `writeClaudeCommand` helper (142–162). **Verify no other
  caller first** (`grep writeClaudeCommand`); if something else uses it, leave it.
- `installClaude` still patches `~/.claude/settings.json` and runs the
  marketplace plugin install — only the loose command write goes away. Confirm
  the function still compiles and reads cleanly (no orphaned `homeDir`-derived
  paths, the `--apply` preview text still makes sense).

### Delete the asset

- Delete `assets/claude/commands/start-task.md`.
- Remove the now-empty `assets/claude/commands/` directory (and `assets/claude/`
  if it becomes empty).

### `install_test.go`

- Remove `TestInstallClaudeWritesStartTaskCommand` (87–106).
- Remove `TestInstallClaudeStartTaskIsIdempotent` (108–131).
- Confirm `filepath` / `strings` imports are still used by the remaining tests
  (they are, elsewhere in the file) so the build stays green.

### `README.md`

- Replace lines 302–303 (the "/start-task command" paragraph) with a short
  pointer telling users the task-kickoff workflow now ships as the
  `domux-start` marketplace plugin, installed alongside the others.

---

## Part C — `/cr` alias for `/codex-review`

The built-in `/review` (PR review) is left untouched. Instead add a short alias.

### Command file

```
plugins/implement-pipeline/commands/cr.md
```

- A **command** (not a second skill) so it fires only when typed, never
  auto-triggers. Body forwards to the skill: instruct the agent to invoke the
  `codex-review` skill on the current changes, passing along any arguments.
- Optionally update the `codex-review` SKILL.md `description` to mention `/cr` as
  an additional trigger phrase (so the agent associates `/cr` with it). Low
  priority; the command file is the mechanism that matters.

This is the implement-pipeline plugin's first `commands/` directory — that's a
standard plugin component, no manifest change needed beyond creating the dir.

---

## Testing & verification

- `go build` — compiles after the `install.go` edits.
- `go test ./...` — passes with the two start-task tests removed.
- `go vet ./...` — clean (catches an orphaned `writeClaudeCommand` if missed).
- Manual sanity: `domux install claude` (preview, no `--apply`) prints the
  settings patch + plugin install plan and **no** "would write start-task"
  line.
- Plugin validity: the three plugins listed in `marketplace.json` each resolve
  to a real directory with the expected `plugin.json` / `SKILL.md` (or
  `commands/`) files.

## Risks / notes

- Anyone who relied on `/start-task` via `domux install claude` *without* adding
  the marketplace must now install the `domux-start` plugin. This is the
  intended trade (single source of truth) — README captures the migration.
- `$ARGUMENTS` adaptation is the one behavioral subtlety; the skill must not
  assume placeholder substitution.
