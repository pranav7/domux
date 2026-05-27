<img width="1140" height="588" alt="domux screenshot" src="https://github.com/user-attachments/assets/9e778d42-f3db-4394-9d70-9a06d6ef66d5" />

# domux

I usually have a few things going at once in a single project: several git
worktrees, each on its own branch, often with a coding agent running in its own
tmux session. tmux keeps all those shells alive, but it doesn't tell me much
about them — which session is on which branch, what I was doing in each one, or
whether the agent over in session 3 is still working or has been sitting there
waiting on me.

I built domux to keep track of all that. It pins each tmux session to the
worktree where it started, gives every session its own todo list, and shows the
whole lot in one switcher: the branch, a label, the task I'm focused on, and
whether an agent is busy, waiting for input, or done. Worktrees do the work
underneath, but I don't have to think about them — I just see what's happening
across my sessions and where my attention is needed.

The part that matters to me is that it lives inside tmux. I didn't want to learn
another tool or keep a browser tab open to track my work. I already work in the
terminal, so domux works there too.

No server, no database. State is plain markdown and JSON under
`~/.local/share/domux`, and the tmux integration is a generated config file you
can read before you source it.

## Requirements

Release binaries are macOS only, Apple Silicon and Intel. You need `tmux` for the
session and switcher workflow; the todo TUI on its own is just a terminal program
that runs anywhere.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

The installer downloads the latest checksum-verified release binary and runs
`domux bootstrap`. No Go toolchain is needed.

The script installs to `~/.local/bin` by default. If that is not on your `PATH`,
it prints the line to add to your shell config.

To inspect first and bootstrap later:

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | DOMUX_SKIP_BOOTSTRAP=1 sh
domux bootstrap
```

Useful installer variables:

| Variable | Meaning |
|---|---|
| `DOMUX_VERSION=v0.1.0` | Install a specific release |
| `DOMUX_INSTALL_DIR=/custom/bin` | Install somewhere else |
| `DOMUX_SKIP_BOOTSTRAP=1` | Skip setup after download |

From source:

```sh
go install github.com/pranav7/domux@latest
domux bootstrap
```

## Bootstrap

`domux bootstrap` prints a plan, asks once, then applies the pieces it can use on
your machine:

- installs tmux with Homebrew if tmux is missing and Homebrew exists
- writes `~/.config/domux/domux.tmux`
- patches Claude Code hooks if Claude Code is present
- patches Codex hooks if Codex is present
- registers idle-sleep prevention with `caffeinate`

The tmux config is not automatically sourced. Add this to `~/.tmux.conf`:

```tmux
source-file ~/.config/domux/domux.tmux
```

Then reload tmux:

```sh
tmux source-file ~/.tmux.conf
```

The generated tmux bindings inherit your existing prefix. `domux` does not change
your tmux prefix.

## Quick Start

Start or resume work in a directory:

```sh
domux start ~/code/my-repo
```

Open the todo TUI for the current context:

```sh
domux
```

Open the session switcher:

```sh
domux switcher
```

Mark the current tmux session as the one running the dev server:

```sh
domux server
```

Give the current session a label:

```sh
domux label set "auth cleanup"
```

Check setup:

```sh
domux doctor
```

## Tmux Bindings

After sourcing `~/.config/domux/domux.tmux`:

| Binding | Action |
|---|---|
| `<prefix> t` | Open todo popup for the current pane path |
| `<prefix> s` | Open session switcher |
| `<prefix> u` | Open utilities popup |
| `<prefix> v` | Toggle server marker for current session |
| `<prefix> N` | Set session label |
| `<prefix> n` | Clear session label |
| `<prefix> i` | Toggle AI state for current pane |
| `<prefix> f` | Clear domux state for current session |

## Todo TUI

`domux` opens the todo list for the current context. Inside a pinned tmux
session, that means the session root. Outside tmux, it uses the current git root
or directory.

| Key | Action |
|---|---|
| `j` / `k` / arrows | Move cursor |
| `a` | Add task |
| `e` | Edit task title |
| `Enter` | Edit notes in `$EDITOR` |
| `Space` / `x` | Mark done and archive |
| `i` | Toggle in progress |
| `f` | Focus task for this session |
| `d` | Delete task |
| `J` / `K` | Move task up or down |
| `Tab` | Show or hide archive |
| `r` | Reload from disk |
| `?` / `h` | Toggle help |
| `q` / `Esc` | Quit |

## Session Switcher

`domux switcher` shows tmux sessions grouped by project. It can show branch and
PR state, labels, focused tasks, server marker, and Claude/Codex activity when
hooks are installed.

| Key | Action |
|---|---|
| `j` / `k` / arrows | Move cursor |
| `Enter` | Switch to selected session |
| `/` | Filter sessions |
| `+` | Create a workspace session in the selected group |
| `n` | Name selected session |
| `c` | Clear selected session |
| `r` | Reset selected branch only |
| `s` | Mark selected session as server |
| `Tab` | Show or hide task previews |
| `D` | Delete workspace sessions or close main sessions |
| `q` / `Esc` | Quit |

## Commands

| Command | Meaning |
|---|---|
| `domux` | Open todo TUI |
| `domux todo` | Same as `domux` |
| `domux switcher` | Open session switcher |
| `domux sessions` | Alias for `switcher` |
| `domux start [DIR]` | Start or resume a pinned tmux session |
| `domux adopt [DIR]` | Pin the current tmux session to a directory |
| `domux attach NAME` | Attach or switch to a tmux session |
| `domux clear` | Reset and free the current workspace |
| `domux reset-branch` | Reset only the current git branch |
| `domux clear-state` | Clear domux state for current session |
| `domux server` | Toggle server marker |
| `domux commands` | Open utilities popup |
| `domux doctor` | Check integration state |
| `domux migrate` | Preview migration from old tmux dotfiles |

Script-friendly output:

| Command | Meaning |
|---|---|
| `domux --path` | Print todo file path for current context |
| `domux --count` | Print active task count |
| `domux --status` | Print tmux status text |
| `domux --list` | Print active tasks |
| `domux --version` | Print version |

## Optional Claude and Codex Hooks

If you use Claude Code or Codex, `domux` can install hooks that update tmux and
the switcher when an agent starts working, waits for input, compacts, or exits.

Preview first:

```sh
domux install claude
domux install codex
```

Apply:

```sh
domux install claude --apply
domux install codex --apply
```

The Claude install also writes a `/start-task` command that tells Claude how to
use domux, tmux sessions, and git worktrees before it starts coding.

There is also an optional Claude Code plugin for an `/implement` pipeline. See
[`docs/implement-pipeline.md`](docs/implement-pipeline.md) if you want that. It
is not required for the normal domux workflow.

## Caffeinate

`domux commands` opens a small utilities popup. The first utility is caffeinate.

Partial mode prevents idle sleep and does not require sudo:

```sh
domux install caffeinate
```

Full mode also prevents lid-close sleep. It requires sudo once during install:

```sh
domux install caffeinate --full
```

## Storage

Todo files are stored as markdown:

```text
~/.local/share/domux/by-path/<escaped-path>.md
```

Example:

```markdown
---
worktree: /Users/alice/code/my-repo
created: 2026-05-18
---

# TODOs

- [ ] Fix login redirect on Safari
  Notes can live under a task.
- [~] Bump react-router

## Archive

- [x] 2026-05-17 - Wire up feature flag
```

Session metadata lives here:

```text
~/.local/share/domux/sessions/<session>.json
```

Generated integration files live here:

```text
~/.config/domux/
```

The files are meant to be readable and recoverable. You can edit the markdown
todo files directly; the TUI reloads changes from disk.

## Migration

Older tmux dotfile state is still read:

- `~/.tmux-label-*`
- `~/.tmux-server-*`
- `~/.tmux-claude-*`
- `~/.tmux-codex-*`
- `~/.tmux-workspace-*`

Preview migration:

```sh
domux migrate
```

Apply migration:

```sh
domux migrate --apply
```

Install commands create backups before writing. Migration does not delete the old
dotfiles.

## Development

```sh
make
make test
make switcher
```

For local development, `make install` symlinks `~/bin/domux` to the repo build:

```sh
make install
```

Remove that symlink with:

```sh
make uninstall
```

## Design Notes

I kept domux deliberately boring: one Go binary, tmux commands, markdown todos,
and small JSON state files. It's meant to make my existing terminal workflow
less ambiguous, not to replace it.
