# 0041: A folder project does not hold the repositories under it

**Date:** 2026-09-13
**Status:** Accepted. Amends decision record 0009's rule for which directories are offered.
Decision records 0006 and 0010 stand.
**Decision:** The attach offer asks about the repository's top level when it is typed inside a
repository, and about the directory itself otherwise. That directory is already held, and
nothing is asked, when the directory attach was typed in is a registered project root or
workspace path or lies under one, with one exception: a folder project does not hold a
repository whose top level lies below the folder. A git project still holds everything under
it. Separately, `project.add` takes a full path. The CLI makes `open` and `project add`
paths full against the directory the command was typed in, and the server refuses a relative
path.

## Context

MUX-37 was reported as "attach is not working on Linux". A fresh 1.0.0 install, and in
`~/domux`:

    > domux attach
    Detached. Run domux to reattach.

with a screen whose Navigator listed one project, `pranav`, and no `domux`. Attach had worked:
the client stayed attached until the detach key. What failed was the offer of 0009, which never
asked.

The first bare `domux` on that machine was typed in a new terminal, which opens in the home
directory, so the start-up seed of 0010 registered `/home/pranav` as a folder project. The offer
skipped any directory that was a registered path or lay under one, whatever the project's kind,
and every repository the author has lies under the home directory. So attach in `~/domux` said
nothing and reconnected to the home project, and `server restart` could not help, because the
seed only runs on an empty model.

The way out the offer names was broken too. `open .` sent `.` to the server as typed, and the
server resolved it against its own directory, which it inherits from whichever command started
it. `open .` answered for that folder with a status of 0, and typed in any other directory it
registered the wrong one. `project add .` did the same. And an offer typed in a subdirectory of
a repository would have registered the subdirectory as a git project of its own.

## Why the kind decides

A git project and a folder project are different things (0010). A git project is one checkout,
and the directories under it are its own: subdirectories, the slots, a worktree made by hand
beside the slots, a submodule. Offering any of them would ask about a project the reader is
already in. A folder project is a place with `main` and no slots, and nothing about a folder
says the repositories under it are part of it. A repository is exactly what `project.add`
registers as a project of its own, and the server already accepts a git project nested under a
folder project.

So a folder project holds the plain folders under it, which keeps a notes directory under a
home project quiet, and a folder record at a repository's top level or inside a repository
still holds, which keeps a record written before 0010 quiet.

## What was not chosen

- Counting a repository as held only when its exact top level is registered. That is exact, and
  it would ask from inside every worktree made by hand and every submodule that is not a slot.
- Not seeding the home directory. The seed exists so a server always has somewhere to be (0010),
  and a server with no project refuses attach, so changing it touches 0009, 0010 and 0021. It is
  left for its own decision. A home project seeded by accident is removed with
  `domux project remove`.
- Registering the top level inside `project.add` itself for any path inside a repository. Only
  the offer chooses the top level; `project add <repo>/crates` still registers `crates`.
- Resolving relative paths only in the CLI. Key bindings and `domux api project.add` reach the
  handler without the CLI's help, so the server refuses what it cannot read correctly, the way
  `git::run` refuses a directory that is not absolute.

## Consequences

- The question names the directory it would register, and the line after a no names it too:
  `Left unregistered. Run domux open <dir> to register it later.` `open .` is only the same
  thing when attach was typed at the top level.
- One `git rev-parse --show-toplevel` in the CLI before the question. It runs in the command a
  person typed, not on the core task. A git that will not answer treats the directory as a plain
  folder.
- A repository under a git project at an ancestor is still held. A home directory that is itself
  a git work tree, seeded as a git project, keeps every repository under it quiet. Nothing
  observed produces one yet.
- `project.add` with a relative path answers `invalid_params` and registers nothing.
