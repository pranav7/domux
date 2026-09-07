# 0006: Attach reconnects to the bound project, it does not follow the invoking directory

**Date:** 2026-09-08
**Status:** Confirmed (the plan's exit criterion already ruled this; recorded here because it
was field-reported as a bug)
**Decision:** `domux2` binds one project to the whole daemon the first time its model is
empty (`Core::new`, `crates/domux-server/src/core.rs`), and every later `attach` or bare
`domux2` reconnects to that same project regardless of the shell's current directory. Running
`domux2 attach` from a second directory does not create a project, a workspace, or even a tab
for it: you land back on the bound project's existing tabs, exactly as `tmux attach` reconnects
you to an existing session's existing windows rather than opening one for wherever you typed
the command.

## Context

Reported as two bugs from a live session: (1) a fresh `domux2` opened its tab named for `$HOME`
rather than the launch directory, with the shell starting in `~`; (2) `domux2 attach` typed
from a second, unrelated directory still landed on the first project. Investigation traced (1)
to a stale `~/.local/share/domux2/state.json` left by an earlier accidental launch from `$HOME`
right after the binary was built - `server stop`/`start` deliberately saves and resumes
structure rather than resetting it, so the bad root persisted across restarts. Clearing that
file and starting fresh from the intended directory produced the correct project name,
workspace path, and pane cwd, which confirms `add_folder_project` and `tab.create`'s cwd
fallback both already do the right thing on a truly empty model.

(2) is not a defect: the M1 plan's exit criterion is verbatim "The author lives in it as a tmux
replacement for **one directory** for a full day, including detach and reattach." One implicit
project for the whole daemon is the scope, not an oversight, and
`Model::add_folder_project(root)` is explicitly "the M1 registration M2 generalizes" per the
plan - multiple projects, each bound to its own directory, is M2's job (`add_git_project`, the
sidebar).

## Consequences

- No code change in M1. A second directory has no representation until M2 ships projects and
  workspaces; until then, working in more than one place means one project's tabs.
- M1 still has no user-facing way to recover from a wrongly-bound project root (no rename, no
  reset short of deleting `state.json` by hand). Worth a look when M2 adds project mutation,
  not before.
