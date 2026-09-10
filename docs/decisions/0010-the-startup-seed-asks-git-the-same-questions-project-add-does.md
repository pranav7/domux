# 0010: The startup seed asks git the same questions `project.add` does

**Date:** 2026-09-09
**Status:** Accepted
**Decision:** When the model is empty, `Core::new` registers the directory the server was
started in through the same reading `project.add` uses: `read_project` for the canonical path,
whether it is a repository, what `origin/HEAD` points at and which `workspace-N` directories
are beside it, then `register_project` for the model change. A directory git will not answer
for is still registered as a folder project.

## Context

Two field reports, one cause.

MUX-7: the sidebar row for the project the server was started in showed no branch, while every
other project's did. MUX-8: worktrees V1 had already made under `.domux/worktrees` were on
disk beside the repository and V2 walked past them, so moving over meant building every
workspace again.

The seed was the one registration nobody types, and it was the one that differed. It called
`Model::add_folder_project` and asked git nothing, which the M1 plan is explicit about: one
implicit plain-folder project was M1's scope, and `add_git_project` and the sidebar were M2's
job. M2 gave `project.add` the full reading and left the seed where it was.

A folder project is not a lesser git project; it is a different thing. `facts::targets` skips
one entirely, which is right for a folder with no branch to have and wrong for a repository:
that is why the row had no branch. And nothing adopts slots for a project that never had a
worktree directory read: that is why the workspaces were not there. The author's own state
recorded both, with `domux` held as `{"kind": "folder"}` at a repository root while `dotfiles`
beside it was a git project showing `master`.

## Consequences

- Three git forks at start-up: `rev-parse`, `symbolic-ref`, `worktree list`. They run in
  `Core::new`, before the core task's loop and before any client can attach, so there is
  nothing for them to hold up. This is the one place a fork is allowed on the way to the core
  task, and it is allowed because the loop has not started.
- The seeded project's root is the canonical path, because that is what `read_project` answers
  with and what `project.add` has always recorded. A server started through a symlinked path
  now registers the path it resolves to.
- A repository that is not a repository any more, or a directory that cannot be read, still
  gets a folder project and a line in the log. The seed exists so a server always has
  somewhere to be; refusing would leave it running and unattachable.
- Records already written are not repaired by the seed, which only runs on an empty model.
  **Superseded in part by decision record 0015:** this record said `project remove --all` was
  the way to start over, and `import v1` against the author's own state showed that to be out of
  proportion to the fault. `project.add` now reconciles a record with the disk, so a folder
  project at a repository becomes a git project with its worktrees adopted the next time the
  path is added, opened or imported. `project remove --all` stands, and is still the way to
  start over deliberately.
