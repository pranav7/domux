# domux

**A tmux switcher that shows which of your parallel AI agents is waiting on you.**

![domux switcher and per-worktree todos](assets/demo.gif)

domux helps me scale multiple sessions in a single project. It's built on top of tmux. I wanted something that extends my current workflow rather learning another tool. I am comfortable working in the terminal, and that's where I wanted this to work. It's simple, and it's not trying to do too much. There are a bunch of alternatives to this of course, but this works best for me, and it's quite scalable, yet simple. The beauty of the world we live in today is building software tailored to you. This is my take on an AI IDE built on top of the terminal. This is currently my daily driver.

There are two main parts to domux, the switcher and the todolist:

### Switcher
<img width="1140" height="588" alt="domux screenshot" src="https://github.com/user-attachments/assets/9e778d42-f3db-4394-9d70-9a06d6ef66d5" />

The switcher <kbd>\<leader\></kbd> + <kbd>s</kbd> is a custom tmux session switcher, but rich with details like the branch a worktree is on, a name for your session, preview of the tasks attached to that session — and shows the status of what AI agent attached to that session is doing,  is it working or waiting for your input. It currently works with claude and codex.

### Todolist
<img width="1160" height="530" alt="image" src="https://github.com/user-attachments/assets/16ca123c-dbc7-44ec-bfc1-b6445ebf35ef" />

The todolist <kbd>\<leader\></kbd> + <kbd>t</kbd>, is a simple tracker to remember what you were doing on that workspace.

### Commands
<img width="962" height="311" alt="image" src="https://github.com/user-attachments/assets/ae7beb8d-d37c-4496-9578-249b8750fbc9" />

There is also a utilities panel <kbd>\<leader\></kbd> + <kbd>u</kbd>, that currently only supports `caffeinate` on / off (and requires sudo in case you want to keep your mac awake even with the lid closed)

That's it, feedback welcome!

------

> _Note: Everything below this point is AI generated to help set up domux on your machine._

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
| `Space` / `x` | Toggle done |
| `A` | Archive task |
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

## Worktree Setup

A new git worktree only checks out tracked files, so gitignored things a worktree
needs to actually work — `CLAUDE.local.md`, `.env`, `node_modules`, build deps —
are missing. A checked-in `.domux/worktree.conf` tells domux what to bring across
and what to run for every worktree it provisions.

```text
# .domux/worktree.conf
link CLAUDE.local.md          # symlink → main checkout (edit once, every worktree sees it)
link .vscode/settings.json    # nested paths ok; works for files and directories
copy .env                     # independent per-worktree copy (preserves mode)
run  npm install              # setup command, runs in the new worktree
```

| Verb | What it does |
|---|---|
| `link <path>` | Symlink `<path>` to the same path in the main checkout. Single source of truth — works for files and directories. |
| `copy <path>` | Copy the file from the main checkout into the worktree (independent, mode preserved). File-only; use `link` or `run cp -r` for directories. |
| `run <cmd>` | Run a shell command in the new worktree. `$DOMUX_MAIN`, `$DOMUX_WORKTREE`, and `$DOMUX_ROOT` are available. |

It runs automatically when the switcher provisions a workspace (`+`), and on demand
against any worktree:

```sh
domux setup [DIR]     # apply <main>/.domux/worktree.conf to DIR (default: cwd)
```

Setup is best-effort: a missing source or a failed step is reported but never aborts
the worktree. `run` failures surface when you call `domux setup` directly, but are
not gated during provisioning (they run live in the new session). Commit
`worktree.conf` (it's team-shared) but keep the worktrees themselves ignored — e.g.
add `/.domux/worktrees/` to `.gitignore`.

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
| `domux setup [DIR]` | Apply `.domux/worktree.conf` to a worktree |
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

## Talk to agents in other worktrees

When you run several worktrees side by side (see `domux start` / workspaces),
the Claude agent in one worktree can message the agent in another — addressing
it **by name**, never by hunting for tmux panes. This powers handoffs like
"I built X on branch Y, please pull it in."

```sh
domux peek                              # list running agents: name, state, task, target
domux send workspace-2 "Pushed the fix on branch eng-225; please rebase onto it."
domux read --lines 80 workspace-2       # read the peer's recent output
```

- **Name** resolves against a session's name, its worktree dir/branch basename,
  or its domux label.
- `domux send` prefixes every message so the peer knows it came from a **peer
  agent, not the human** (`--from NAME` overrides the attribution). The text is
  sent literally through tmux, so quotes, backticks, and paths need no escaping.
- By default a message sent to a busy peer is **queued** by Claude Code; pass
  `--wait` to block until the peer is idle first, or `--no-enter` to stage the
  text without submitting it.

The same workflow is available to agents as the `/domux-communicate` plugin
skill (`plugins/domux-communicate`).

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

The task-kickoff workflow (set up the worktree, branch off fresh `main`, label
the session, then start coding) ships as the `domux-start` Claude Code plugin,
installed from the domux marketplace alongside the others. Invoke it with
`/domux-start <task>`.

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
- [x] 2026-05-18 — Verify deploy

## Archive

- [x] 2026-05-17 — Wire up feature flag
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
