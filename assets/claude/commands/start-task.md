Start a new task: **$ARGUMENTS**

You're being kicked off in a tmux+domux workbench. Set up the workspace before touching code.

## What domux is

`domux` pins each tmux session to a git **worktree root** and tracks per-session
state (label, AI status, focused TODO). Worktrees are independent checkouts of
the same repo at different paths — typically one per in-flight task — so each
session is meant to be a fresh, short-lived branch off `origin/main`.

- Session is pinned to the worktree root, so `cd`ing into subdirs is fine.
- TODO list lives at the path printed by `domux --path`.
- Session label shows in the switcher; set it so the human can find you.

**Restricted branches — never commit directly to:** `main`, `master`, `workspace-*`.

## Setup steps (do these before anything else)

1. **Resolve the workspace.**
   - `git rev-parse --show-toplevel` → worktree root.
   - `git worktree list` → check whether this checkout is the primary or a worktree.
   - `git status --porcelain` → check for uncommitted changes.

2. **Refresh main.** `git fetch origin main`.

3. **Branch off fresh main.** Pick a kebab-case branch name from the task
   (e.g. `fix-login-redirect`, `add-export-csv` — short, 2–5 words).
   - **If this is a worktree AND it's clean:** `git checkout main && git reset --hard origin/main && git checkout -b <branch>`. Worktrees are meant to be wiped between tasks.
   - **If this is the primary checkout OR there are uncommitted changes:** do NOT reset. Just `git checkout -b <branch> origin/main` (or stop and ask the user how to handle the dirty state).

4. **Label the session** so it shows up in the domux switcher:
   `domux label set "<2–4 word task title>"`. Current tmux session is auto-detected.

5. **Acknowledge in one line**, then start the task: $ARGUMENTS

## domux quick reference

| Command | What it does |
|---|---|
| `domux --path` | Print the pinned TODO file for this session |
| `domux label set "..."` | Name the current session (shown in switcher) |
| `domux label clear` | Clear the session name |
| `domux sessions` | Open the session switcher TUI |
| `domux todo` | Open the per-worktree TODO TUI |
| `domux --status` | Top active task (for status bars) |

Sessions, labels, and AI state are all auto-managed by hooks — you only need
to set the label and pick the branch. Everything else just works.
