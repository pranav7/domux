<img width="1140" height="588" alt="image" src="https://github.com/user-attachments/assets/9e778d42-f3db-4394-9d70-9a06d6ef66d5" />

# domux

An opinionated tmux workbench. Two TUIs, one binary:

- **todo** — per-worktree task list (the "do" in *do*mux). Pinned to the git
  root where work started, so `cd`-ing through subdirs doesn't lose context.
- **switcher** — pinned-session picker across all your tmux sessions, with
  live AI status, labels, and inline task previews.

## Usage

```sh
# Open the todo TUI for the current domux context
domux
domux todo            # explicit alias

# Open the session switcher
domux switcher
domux sessions        # historical alias

# Start or resume tmux work in this directory
domux start

# Start or resume tmux work in another directory
domux start ~/code/my-repo

# Adopt an existing tmux session into domux
domux adopt

# Attach or switch to an existing session
domux attach my-repo

# Reset/free the current tmux workspace, same as trr/tmux-reset
domux clear

# Reset only git branch, keeping domux session state
domux reset-branch

# Clear only domux session state, without git reset behavior
domux clear-state

# Toggle the current tmux session as running the server
domux server

# Preview installable tmux, Claude Code, and Codex integration
domux install tmux
domux install claude
domux install codex

# Register caffeinate (partial mode — idle-sleep prevention, no sudo)
domux install caffeinate

# One-shot setup: detect brew/tmux/Claude/Codex and apply hooks
domux bootstrap

# Open the utilities popup (toggle caffeinate, …)
domux commands

# Check installed integration state
domux doctor

# Print storage path for current context
domux --path

# Show help
domux --help
```

## Installation

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

macOS only (Apple Silicon + Intel). No Go toolchain required — the script
downloads the latest checksum-verified release binary into `~/.local/bin`
and then hands off to `domux bootstrap`.

Env var overrides: `DOMUX_VERSION=v0.1.0`, `DOMUX_INSTALL_DIR=/custom/path`,
`DOMUX_SKIP_BOOTSTRAP=1`.

### From source

```sh
go install github.com/pranav7/domux@latest
domux bootstrap
```

### One-shot bootstrap

`domux bootstrap` detects Homebrew, tmux, Claude Code, and Codex; prints
the plan; asks once for confirmation; then writes the tmux integration,
patches Claude and Codex hooks if those tools are present, and registers
caffeinate in partial mode (idle-sleep prevention only — no sudo). For
lid-close sleep prevention (requires sudo), run
`domux install caffeinate --full` separately.

The generated `bind-key` entries inherit whatever tmux prefix you already
use — domux does not change your prefix.

### Local development

```sh
make           # build ./domux
make install   # symlink ~/bin/domux -> ./domux (one-time)
make test
make switcher  # quick launch for testing
```

After `make install`, every subsequent `make` is live on your PATH — no
copy step. Use `make uninstall` to remove the symlink (the original
binary, if any, was backed up to `~/bin/domux.pre-symlink.bak`).


Preview the generated tmux integration:

```sh
domux install tmux
```

When ready, write it:

```sh
domux install tmux --apply
```

Then add this line to `~/.tmux.conf`:

```tmux
source-file ~/.config/domux/domux.tmux
```

Claude Code integration is also preview-first:

```sh
domux install claude
domux install claude --apply
```

This patches `~/.claude/settings.json` with AI-state hooks **and** writes a
`/start-task` slash command to `~/.claude/commands/start-task.md`. Inside
Claude Code, run:

```
/start-task fix login redirect on Safari
```

The command teaches Claude about domux + git worktrees, then has it refresh
`origin/main`, branch off, and set the domux session label before touching
code. Restricted branches (`main`, `master`, `workspace-*`) are noted so
Claude won't commit to them.

Codex integration is preview-first too:

```sh
domux install codex
domux install codex --apply
```

Install commands create backups before writing and do not delete legacy
`~/.tmux-*` state files.

## Commands popup

`<prefix> u` opens a small popup with toggleable utilities:

```sh
domux commands
```

First entry is caffeinate. Enter toggles it. Partial mode runs
`caffeinate -dimsu` in the background (idle-sleep only). Full mode also
manages a launchd daemon and `pmset disablesleep` — enable it via
`domux install caffeinate --full` (requires sudo once at install time).

## Switcher keybindings

| Key | Action |
|-----|--------|
| `j` / `k` / arrows | Move cursor |
| `Enter` | Switch to selected session |
| `n` | Name (label) the selected session inline |
| `c` | Clear/reset selected session |
| `r` | Reset selected session branch only |
| `s` | Mark selected session as running the server |
| `Tab` | Show/hide todos |
| `/` | Filter sessions |
| `q` / `Esc` | Quit |

## Features

- One TODO list per git worktree
- Tmux sessions pinned to their starting root
- Focused TODO per session
- Native terminal TUI (bubbletea)
- File-backed markdown format
- Claude Code and Codex hooks can update tmux AI state through domux
- Live reload via fsnotify
- Vim-style keybindings

## Todo keybindings

| Key | Action |
|-----|--------|
| `j` / `k` / arrows | Move cursor |
| `a` | Add new task |
| `e` | Edit selected task |
| `i` | Toggle in progress / restore archived task in progress |
| `f` | Focus selected task for current session |
| `o` | Reopen archived task |
| `Enter` | Edit notes in $EDITOR |
| `Space` / `x` | Toggle done (moves to archive) |
| `d` | Delete task |
| `J` / `K` | Move task up/down |
| `Tab` | Expand/collapse archive |
| `r` | Reload file from disk |
| `?` / `h` | Toggle help overlay |
| `q` / `Esc` | Quit |

## Data format

TODO files are stored at `~/.local/share/domux/by-path/<sanitized-path>.md` in GitHub-flavored markdown:

```markdown
---
worktree: /Users/pranav/code/audrey-app
created: 2026-05-18
---

# TODOs

- [ ] Fix login redirect on Safari
  Stack trace pointed at auth-callback.tsx:42.
- [~] Bump react-router to v7

## Archive

- [x] 2026-05-17 — Wire up new feature flag for X
```

Domux lazily adds stable HTML-comment IDs when a TODO file is saved:

```markdown
- [ ] Fix login redirect on Safari <!-- domux:id=1a2b3c4d5e6f7890 -->
```

Session metadata is stored in `~/.local/share/domux/sessions/<session>.json`.
Generated integration files live under `~/.config/domux/`.

## Claude interface

To find the current session's pinned list:

```sh
domux --path
# prints: /Users/pranav/.local/share/domux/by-path/%2FUsers%2Fpranav%2Fcode%2Faudrey-app.md
```

Claude can read/write that path directly. The TUI picks up edits via fsnotify.

## Migration

Existing dotfiles state is still read:

- `~/.tmux-label-*`
- `~/.tmux-server-*`
- `~/.tmux-claude-*`
- `~/.tmux-codex-*`
- `~/.tmux-workspace-*`

Use dry-run migration first:

```sh
domux migrate
```

Apply only after reviewing the output:

```sh
domux migrate --apply
```

## Design

The implementation is intentionally self-contained so it can be installed
outside personal dotfiles and adopted gradually.
