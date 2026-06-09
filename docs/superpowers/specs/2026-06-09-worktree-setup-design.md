# Per-worktree setup (`.domux/worktree.conf`)

**Date:** 2026-06-09
**Status:** Spec — pending implementation

## Goal

When domux provisions a new `workspace-N` worktree, automatically bring across the
files and run the setup a worktree needs to *function* — chiefly gitignored files
that never appear in a fresh worktree (e.g. `CLAUDE.local.md`, `.env`) plus optional
setup commands (`npm install`). Driven by a checked-in, team-shared config so every
worktree in a project is provisioned identically.

## Background / motivation

A git worktree only checks out **tracked** files. Anything gitignored in the main
checkout — `CLAUDE.local.md`, `.env`, `node_modules` — is absent in a new worktree,
so the worktree can't function without manual setup. domux already creates worktrees
(`provisionWorkspace` in `workspaces.go`, triggered by the picker's "new workspace in
group" action), landing them at `<root>/.domux/worktrees/workspace-N`. That's the
natural place to hook setup.

Prior art for the two halves of this problem:
- **Run setup → "post-create command":** devcontainer.json `postCreateCommand`
  (VS Code / Codespaces), npm `postinstall`, Makefile `setup`, AI-worktree tools
  (Crystal, uzi). Dominant, widely understood.
- **Bring untracked files across → declarative link list:** dotfiles managers
  (GNU Stow, chezmoi, rcm).

This feature combines both in one line-oriented config.

## Non-goals

- Personal/global override config layered on top of the project config. Single
  checked-in file only for v1; a `~/.config/domux/` override can come later.
- Recursive directory `copy`. `copy` is file-only; directories use `link` (symlink)
  or `run cp -r`.
- Exit-code capture / gating on `run` commands during picker provisioning (they run
  in the new tmux session — see Execution model).
- Templating / variable interpolation inside the conf beyond the documented `run`
  environment variables.

## Config file

**Location:** `.domux/worktree.conf` in the main checkout, alongside the existing
`.domux/worktrees/` dir.

**Tracked in git** (team-shared). The worktrees themselves stay ignored — `.gitignore`
should ignore `/.domux/worktrees/` specifically rather than all of `/.domux/`. (If a
project currently ignores all of `/.domux/`, narrow it so `worktree.conf` is tracked.)

**Format:** line-oriented, parsed without a markdown/TOML library (matches `loadList`
in `store.go`). Blank lines and `#`-prefixed lines ignored. Each directive is
`<verb> <argument>`:

```
# .domux/worktree.conf
link CLAUDE.local.md          # symlink worktree/<path> -> <main>/<path>
link .vscode/settings.json    # nested paths ok; parent dirs created in worktree
copy .env                     # independent per-worktree copy (preserves file mode)
run  npm install              # setup command, runs in the new worktree
```

### Verbs

| Verb | Behavior |
|---|---|
| `link <path>` | Create `worktree/<path>` as a symlink whose target is the **absolute** path `<main>/<path>`. Works for files and directories (`link node_modules` is the cheap trick). Single source of truth — editing the main copy is reflected in every worktree. Parent dirs of `<path>` are created in the worktree as needed. |
| `copy <path>` | Copy `<main>/<path>` to `worktree/<path>` as an independent file, preserving mode. File-only. |
| `run <cmd...>` | Run an arbitrary shell command (the devcontainer-`postCreateCommand` escape hatch). Execution context depends on entry point — see Execution model. |

Unknown verbs are a parse warning (skipped), not a fatal error, so older domux binaries
degrade gracefully against a newer conf.

## When it runs

1. **Automatically on provision.** At the end of `provisionWorkspace()`, after the
   worktree and tmux session exist, domux applies `worktree.conf`.
2. **Manually via `domux setup [--path DIR]`.** A new subcommand applies
   `worktree.conf` to an existing worktree (covers worktrees created by hand with
   `git worktree add`, and re-running after editing the conf). `--path` defaults to
   the cwd. Provisioning calls this same internal code path.

### Resolving the main checkout

The config and all `link`/`copy` sources are read from the **main checkout**, resolved
from the target worktree path:
- If the path is under a known worktree dir, strip the
  `.domux/worktrees/workspace-N` (or legacy `.baag/worktrees/...`) suffix — reuse
  `workspaceRootFromPath` in `workspaces.go`.
- Otherwise, fall back to the main worktree reported by `git worktree list`
  (first entry).

If no `.domux/worktree.conf` exists at the resolved main, setup is a no-op (silent for
provisioning; a friendly note for the explicit `domux setup` command).

## Execution model & error handling

- **`link` / `copy`:** synchronous Go filesystem operations, performed during
  provisioning **before** anything long-running. Fast, deterministic, fully unit-testable.
- **`run`:**
  - From **picker provisioning**: each `run` command is sent into the new tmux session
    via `tmux send-keys` (in conf order), so a slow `npm install` does not freeze the
    picker TUI and the developer watches setup happen in their fresh session.
  - From **`domux setup` in a plain terminal**: each `run` command is exec'd inline,
    streaming to stdout (blocking is acceptable in a foreground terminal).
- **Environment for `run`:** commands run with cwd = the worktree and these env vars
  exported: `DOMUX_MAIN` (main checkout path), `DOMUX_WORKTREE` (the worktree path),
  `DOMUX_ROOT` (group root, == `DOMUX_MAIN`). So `run ln -sf "$DOMUX_MAIN/foo" .` works
  for cases the declarative verbs don't cover.
- **Best-effort, non-fatal:** a missing `link`/`copy` source (e.g. `CLAUDE.local.md`
  not yet created) or a failed step emits a warning but never aborts the already-created
  worktree. Collected outcomes are surfaced in the picker status line / on stdout.

## Implementation sketch

New file `worktree_setup.go`:
- `type setupDirective struct { Verb, Arg string }`
- `parseWorktreeConf(r io.Reader) ([]setupDirective, []string)` — directives + warnings.
- `applyWorktreeSetup(main, worktree string, runner runFunc) []setupResult` — performs
  `link`/`copy` synchronously, dispatches `run` through an injected `runner` so tests
  assert issued commands without a real tmux/shell.
- Link/copy helpers honoring the atomic-write / `os.Rename` convention where it applies
  (copy writes `path+".tmp"` then renames), `os.MkdirAll(0755)` for parent dirs,
  `os.Symlink` for link (removing a pre-existing path first, idempotent re-run).

Wiring:
- `provisionWorkspace` (`workspaces.go`): after `setSessionRoot`, resolve main, parse
  conf, call `applyWorktreeSetup` with a tmux-send-keys runner targeting `res.Session`.
  Surface a short summary in `workspaceResult` for the picker status line.
- New `setup` case in `runCommand` (`commands.go`) using
  `flag.NewFlagSet("setup", flag.ContinueOnError)` with `--path`; runner = inline exec.
- Register `setup` in help output (`main.go`).

## Testing

`go test` unit coverage:
- `parseWorktreeConf`: comments, blanks, each verb, unknown verb → warning, malformed
  lines.
- `applyWorktreeSetup` against a temp main+worktree pair: `link` (file, nested path,
  directory), `copy` (mode preserved), missing source → warning (non-fatal), idempotent
  re-run, `run` dispatched to the injected runner in conf order with correct cwd/env.
- `domux setup --path` end-to-end on a temp repo: no conf → friendly no-op; conf present
  → files materialized.
- Main-checkout resolution: from a `workspace-N` path and from a plain worktree.

## Decisions made during brainstorming

| Decision | Choice |
|---|---|
| Mechanism | One checked-in `.domux/worktree.conf` with `link` / `copy` / `run` verbs (declarative file ops + post-create command escape hatch) |
| Config location | `.domux/worktree.conf` in main checkout; ignore `/.domux/worktrees/`, track the conf |
| `run` execution | Picker → `tmux send-keys` into the new session (non-blocking, visible); `domux setup` CLI → inline exec |
| Failure policy | Best-effort, non-fatal; warn and continue, never abort the created worktree |
| `link` vs `copy` | `link` = symlink (files + dirs, single source of truth); `copy` = file-only independent copy |
| Manual entry point | New `domux setup [--path DIR]` subcommand; provisioning reuses its code |
| `run` environment | cwd = worktree; `DOMUX_MAIN` / `DOMUX_WORKTREE` / `DOMUX_ROOT` exported |
