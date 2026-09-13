# 0041: A folder project does not hold the repositories under it

**Date:** 2026-09-13
**Status:** Accepted. Amends decision record 0009's rule for which directories are offered.
Decision records 0006 and 0010 stand.
**Decision:** The attach offer asks about the repository's top level when it is typed inside a
repository, and about the directory itself otherwise. That directory is already held, and
nothing is asked, when the directory attach was typed in is a registered project root or
workspace path or lies under one, with one exception: a folder project does not hold a
repository whose top level lies below the folder. A git project still holds everything under
it, and it also holds a linked worktree of its repository wherever the worktree was made: a
repository whose common git directory is a registered git project's is held. The offer is best
effort, so anything that stops it short is said in one line and the attach goes on.
Separately, `project.add` takes a full path. The CLI makes `open` and `project add`
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

## A linked worktree is its project's

A review of the first change found the other half of the same nuisance. `git worktree add
../app-feature` puts a worktree beside the checkout rather than under it, so no registered path
holds it, and the offer asked there on every attach. 0009 keeps no record of a no, so there was
no way to make it stop short of registering a second git project out of one repository.

Every work tree of a repository shares one common git directory: `.git` in the checkout, which a
linked worktree's `.git` file points back to. So the CLI asks git for the common directory of
the repository it was typed in and of each registered git project's root, and a match is held.
It asks with `git rev-parse --path-format=absolute --git-common-dir`. A git older than 2.31 does
not know `--path-format` and prints the flag back rather than refusing it, so it is asked again
without the flag and its relative answer is read from the directory it was asked in. Only a git
project's root is asked: a slot shares its project's common directory, and a folder project is
not a repository's project (0010). `at_home` takes paths the caller has already resolved and
asks for common directories through a function the caller passes, so its tests answer for git.

The same check could go the other way and offer a submodule or a clone vendored under a git
project, since each has a common directory of its own. It does not, and a git project still
holds everything under its root. A submodule is checked out by its superproject and worked on
as part of it, and a clone under a project's root is most often something the project uses, a
dependency or a fixture. Neither is often a project of its own, and the offer has no record of
a no, so asking there would ask on every attach in a directory the author is working in. The
common directory only ever adds to what is held, so nothing this rule kept quiet before is
offered now.

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
  thing when attach was typed at the top level. The line is for pasting, so a directory a shell
  would split or expand is single quoted in it, by the one helper the hook installer also uses.
- A folder project registered over a tree of repositories, such as `domux open ~/code`, now
  asks in each repository under it, on every attach there, until each is registered. Before,
  the folder held them all and nothing was asked. Removing the folder project with
  `domux project remove` does not stop it; registering the repositories does.
- Every work tree of a registered git project's repository is held wherever it was made: a
  linked worktree beside the checkout, and the checkout itself when what was registered is a
  linked worktree. A second clone of the same remote has a common directory of its own and is
  offered.
- Before the question, the CLI runs `git rev-parse --show-toplevel`. Only when no registered
  path holds the directory and it is in a repository does it ask for the repository's common
  directory, and then for each registered git project's root until one matches. The CLI waits
  on each git process in steps of 10 ms, so each costs about 10 ms: attach typed inside a
  registered project costs one, and a repository no path holds costs one more for itself and
  one for each root asked, twice that on a git older than 2.31.
  These run in the command a person typed, not on the core task. A git that will not answer
  treats the directory as a plain folder, and a root git will not answer for matches nothing
  and is not asked twice.
- A repository under a git project at an ancestor is still held. A home directory that is itself
  a git work tree, seeded as a git project, keeps every repository under it quiet. Nothing
  observed produces one yet.
- The offer never ends the attach. A server that will not answer `project.list`, a
  `project.add` it refuses and a switch that fails are each said in one line that names the
  directory and the `domux open` that does it, and then the client attaches. A directory whose
  path is not UTF-8 is not asked about, because a path reaches the server as a JSON string, and
  the line says so. A reader who is not on a terminal is still not asked and told nothing.
- `project.add` with a relative path answers `invalid_params` and registers nothing. An empty
  path and a path that starts with `~` are refused the same way, each with its own sentence:
  one is no path at all, and the other was written for a shell that never saw it. `open` and
  `project add` typed in a directory whose path is not UTF-8 say so and exit 1, because the
  full path they now send cannot be written as a JSON string.
