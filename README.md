# domux

An opinionated tmux workbench with a session picker and per-session TODOs.

Domux pins a tmux session to the directory where the work started. The TODO
context stays attached to that session even if an agent or shell later `cd`s
through subdirectories.

## Usage

```sh
# Open the TODO TUI for the current domux context
domux

# Start or resume tmux work in this directory
domux start

# Start or resume tmux work in another directory
domux start ~/code/my-repo

# Adopt an existing tmux session into domux
domux adopt

# Attach or switch to an existing session
domux attach my-repo

# Launch the session picker
domux sessions

# Reset/free the current tmux workspace, same as trr/tmux-reset
domux clear

# Clear only domux session state, without git reset/merge behavior
domux clear-state

# Toggle the current tmux session as running the server
domux server

# Preview installable tmux and Claude Code integration
domux install tmux
domux install claude

# Check installed integration state
domux doctor

# Print storage path for current context
domux --path

# Show help
domux --help
```

## Installation

```sh
go install github.com/pranav7/domux@latest
```

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

Install commands create backups before writing and do not delete legacy
`~/.tmux-*` state files.

## Session picker keybindings

| Key | Action |
|-----|--------|
| `j` / `k` / arrows | Move cursor |
| `Enter` | Switch to selected session |
| `c` | Clear/reset selected session |
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
- Claude Code hooks can update tmux AI state through domux
- Live reload via fsnotify
- Vim-style keybindings

## Keybindings

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
