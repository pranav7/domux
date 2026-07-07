# Changelog

All notable changes to domux are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`domux resume`** — recreate saved tmux work sessions after a restart. Bare
  `domux resume` restores every saved session; `domux resume <project>`
  restores just one group. Progress shows in the switcher, which you land in so
  you can see what came back. Sessions whose worktree directory is gone are
  skipped and their stale state files pruned.
- **`DOMUX_DEBUG` trace log** — set `DOMUX_DEBUG=1` (default file
  `~/.local/share/domux/debug.log`) or `DOMUX_DEBUG=/path/to.log` to trace the
  session attach path. Off by default.

### Removed

- **`implement-pipeline` plugin** — the generic `/implement` and `/codex-review`
  loop no longer ships from the domux marketplace; it now lives in the user's
  dotfiles as Claude skills. domux keeps only its own `domux-start` and
  `domux-communicate` plugins, and `domux install claude --apply` installs those.

### Fixed

- **Switcher no longer corrupts the terminal when launched from a plain shell.**
  Selecting a session ran `tmux attach-session` while the picker still held the
  terminal, so the two fought over the tty and left it unusable (most visible
  via `domux resume` after a reboot). The attach now happens only after the
  picker has exited and released the terminal.

## [0.2.0] - 2026-06-25

A feature release: per-worktree setup, inter-worktree agent messaging, and a
Claude Code plugin suite (task kickoff, the `/implement` pipeline, and external
review).

### Added

- **Per-worktree setup** — `domux setup` reads a `.domux/worktree.conf` and
  links, copies, or runs commands to provision a new worktree. Setup is also
  applied on provision and surfaced in the picker status. Accepts a positional
  directory; refuses to set up the main checkout into itself.
- **Inter-worktree agent messaging** — `domux send` / `domux read` /
  `domux peek` let the Claude agent in one worktree message, read, and discover
  the agent in another. Resolves a peer by session name, directory/branch
  basename, or label; supports `--from`, `--pane`, `--no-enter`, and
  `--wait`. Messages are peer-attributed so the receiving agent knows they came
  from another agent.
- **`domux-communicate` plugin** — ships the messaging workflow as a Claude Code
  plugin skill in the domux marketplace.
- **`domux-start` plugin** — the task-kickoff workflow (resolve the worktree,
  branch off fresh `main`, label the session, then start) now ships as a
  marketplace plugin skill that auto-triggers on task kickoff.
- **`/implement` pipeline plugin** — hands-off implement → simplify → lint →
  analyze → PR, reusing Claude's built-in skills.
- **`/codex-review` skill** — external second-opinion review of the current
  changes via Codex (GPT-5.5 on Azure), plus a parallel analyze fan-out in the
  `/implement` pipeline.
- **`/cr` alias** — short alias for `/codex-review`. The built-in `/review`
  (PR review) is left untouched.

### Changed

- **Retired the embedded `/start-task` command.** It is replaced by the
  `domux-start` plugin; `domux install claude` no longer writes
  `~/.claude/commands/start-task.md`.
- Recap is hidden after a clear until a fresh one is written.
- The session picker wraps the session recap instead of truncating it.
- `/implement` slimmed to a single skill that reuses Claude's built-ins.
- CI actions bumped to node24-ready majors.

## [0.1.4] - 2026-06-05

Baseline for this changelog. See the
[v0.1.4 release](https://github.com/pranav7/domux/releases/tag/v0.1.4) and
earlier tags for prior history.

[0.2.0]: https://github.com/pranav7/domux/compare/v0.1.4...v0.2.0
[0.1.4]: https://github.com/pranav7/domux/releases/tag/v0.1.4
