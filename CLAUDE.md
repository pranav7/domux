# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & test

```sh
go build              # produces ./domux
go test ./...         # run all tests
go test -run TestX    # single test
go vet ./...
```

Single-package layout (`package main`) — no module subdirs. Go 1.22.

## Architecture

**Two surfaces, one binary.** `main.go` dispatches: bare `domux` (or flags like `--path`/`--list`) goes through `flag.Parse` then `runTUI`; anything starting with a non-`-` arg routes through `runCommand` in `commands.go` to subcommands (`start`, `attach`, `sessions`, `clear`, `install`, `ai-state`, …).

**Context resolution is the spine.** Every command path funnels through `resolveDomuxContext(cwd)` in `session.go`. It tries the current tmux session first (`resolveDomuxContextForSession`), falling back to path-based lookup. Session state, when present, pins a session to a `Root` (git worktree root) so the TODO file stays attached even after `cd` into subdirs. The returned `DomuxContext` carries `Session`, `Root`, `TodoPath`, and `State`.

**Storage layout** (all under `$HOME`):
- `~/.local/share/domux/by-path/<sanitized-root>.md` — markdown TODO file per worktree root. `/` → `%2F`. See `resolver.go`.
- `~/.local/share/domux/sessions/<session>.json` — `SessionState` (root, todo_path, focused id, label, server, workspace, AI map). See `session.go`.
- `~/.config/domux/domux.tmux` — generated tmux integration written by `domux install tmux`.

**Legacy dotfile bridge.** `mergeLegacyState` in `session.go` reads `~/.tmux-label-*`, `.tmux-server-*`, `.tmux-workspace-*`, `.tmux-claude-*`, `.tmux-codex-*` and folds them into `SessionState` on load. `setAIState`, `labelCommand`, etc. also *write* these legacy files so old tmux configs keep working. Don't remove the legacy writes without a migration plan.

**AI state model.** `state.AI` is `map[string]string` keyed `agent:pane` (e.g. `claude:1_0`, `codex:default`). `aggregateAIStatesFromSession` collapses per-pane entries into `AIStates{Claude, Codex}` with WAITING winning over CLAUDING/CODEXING. `normalizeAIState` is the single source of truth for value canonicalization — go through it.

**TODO file format.** GitHub-flavored markdown with YAML frontmatter. Active `- [ ]` / in-progress `- [~] ` / done `- [x] YYYY-MM-DD — title`; archived done items live under `## Archive`. Lazy stable IDs appended as `<!-- domux:id=… -->` on save (`store.go`). Parsing is line-oriented in `loadList`; preserve that — no markdown library.

**TUIs.** Two bubbletea models:
- `tui.go` — per-session TODO editor. fsnotify-backed live reload (`watcher`, `pendingReload`).
- `picker.go` — session picker with periodic refresh (`pickerRefreshInterval`). Has a 150ms startup grace where Esc is ignored (avoids inherited keystrokes — see `TestPickerIgnoresInitialEscape`).

Both use the Catppuccin Mocha palette declared in `tui.go`.

**Install commands are preview-first.** `domux install tmux|claude|codex` prints what *would* be written; `--apply` performs the write and always calls `backupIfExists` first. `patchedClaudeSettings` / `patchedCodexHooks` mutate user settings idempotently via `addCommandHook` (skips dupes by command string). `pruneCopiedClaudeCodexHooks` removes copy-paste-from-Claude residue in Codex configs — keep that pruning when extending.

**Tmux interop is `exec.Command` only.** No library — just shell out (`currentTmuxSession`, `tmuxSessionExists`, `attachTmuxSession`, etc.). After mutating session state that affects the status bar, call `refreshTmuxClient` (`tmux refresh-client -S`).

**Claude usage provider (`usage_source.go`).** The only outbound HTTP in the repo: reads the Claude OAuth token (env `DOMUX_CLAUDE_TOKEN` → `~/.claude/.credentials.json` → macOS Keychain via `security`) and calls `GET /api/oauth/usage`. All fragile external contact — endpoint URL, `anthropic-beta` value, Keychain service/account, and the response shape — is isolated here so a schema change is a one-file fix; `usage.go`/`picker.go` see only the normalized `UsageSnapshot`. The response's canonical source is the self-describing `limits` array (`{kind, percent, resets_at, scope.model.display_name}`); `parseUsage` reads it first (the Fable window is the `weekly_scoped` entry — the flat `seven_day_opus` field is `null` live), falling back to the flat `five_hour`/`seven_day` fields only if `limits` is absent. Two invariants: **never log/persist/render the token**, and **never fabricate a number on schema drift** — `percent` is a pointer and a window is skipped when it's absent (an absent value must not become `0%`); unknown limit `kind`s are skipped, not mislabeled. Failures return a sentinel/generic error the UI renders as an honest "unavailable" state. `DOMUX_USAGE_FIXTURE` renders a captured JSON with no network; `domux usage --raw` prints the raw (token-free) response body for re-pinning field names.

## Conventions

- Atomic writes: write to `path + ".tmp"` then `os.Rename` (see `saveList`, `saveSessionState`). Use this pattern for any new on-disk state.
- Session-name sanitization: `sanitizeSessionName` (`/` → `%2F`, `:` → `%3A`) for state filenames; `cleanSessionName` for *tmux* session names (lowercase, dashes only).
- When adding a subcommand, register it in `runCommand` and use `flag.NewFlagSet(name, flag.ContinueOnError)` with `fs.SetOutput(os.Stderr)`.
