# 0015: `project.add` reconciles a record with the disk

**Date:** 2026-09-09
**Status:** Accepted. Replaces the last consequence of decision record 0010, whose decision
stands.
**Decision:** `project.add` on a path that is registered already brings the record up to what
`read_project` just read: a folder record at a path git answers for becomes a git project with
the branch `origin/HEAD` points at, and every `workspace-N` worktree beside the root that the
model does not hold is adopted. It still answers with the project the path already is, and
`adopted` still names only the slots this call registered.

## Context

Decision record 0010 gave the start-up seed the same reading `project.add` does, and recorded
as a consequence that records already written are not repaired: "A folder project in an
existing `state.json` stays one, because the seed only runs on an empty model. `project remove
--all` is the way to start over."

`import v1` is what made that consequence unaffordable. Run against the author's own V1 state
on 2026-09-09 it reported:

    /Users/pranav/projects/audrey/audrey-app/.domux/worktrees/workspace-1 is not registered
    under /Users/pranav/projects/audrey/audrey-app, so its tabs were not created.

with the same line for `workspace-2` and `workspace-3`, and a failure line ending "Read the
messages above, then run this again." The server had been started in `audrey-app` by a build
from before 0010, so the seed had registered it as `{"kind": "folder"}` with `main` and none of
the four worktrees on disk beside it. `project.add` answered for that record as it stood,
`workspace.list` held no slot to rename, and three workspaces and their tabs did not arrive.

Running it again could never have helped, which is the part that decided this. The remedy 0010
names is `project remove --all`, and it is out of proportion: it takes every name, tab, pane
layout and saved directory in every project to correct one project's kind. The reader is not
told about it either, and there is no reason they would connect a sentence about one worktree
to a command that empties the server.

## Why `project.add` and not start-up

Adopting at start would repair without anyone typing anything, and it is the symmetry
`prune_missing_paths` suggests: that already reconciles in one direction, removing records whose
path is gone. It was not taken because it costs three git forks per project at every start,
where 0010's three are once, and because it is not needed. `project.add` is the call `open`
makes, the call the attach offer of decision record 0009 makes, and the call `import v1` makes
for every project it plans, so the reader who has a record to correct reaches it by doing the
thing they were going to do anyway.

## Consequences

- The kind only ever gains. A folder at a path git now answers for becomes a git project, and a
  git project whose `origin/HEAD` has moved records the branch it moved to. A git project at a
  path git has stopped answering for keeps its kind: its slots are recorded, a folder project
  has none, and taking the kind away would leave slot records that `facts::targets` draws no
  branch for. A path that is gone altogether is still `prune_missing_paths`'s, at start.
- `import v1` repairs as it goes, because it calls `project.add` for every project it plans.
  Nothing was added to the import for this.
- `adopted` is unchanged in meaning: what this call registered, never what it found on disk. A
  second `project.add` against a project whose worktrees are all registered still answers `[]`,
  and `adding_a_repository_registers_main_and_adopts_the_worktrees_on_disk` still pins that.
- One implementation adopts: `Core::adopt_slots`, which skips the slots the model holds and is
  called by `register_project` for a new path and by `reconcile_project` for a registered one.
  For a new path the skip is a no-op, so the two callers cannot come to disagree about what
  adoption is.
- A slot the model holds at a path the worktree has since moved from is not corrected here.
  Nothing observed produces one, and `prune_missing_paths` takes the record at the next start
  if the old path is gone.
