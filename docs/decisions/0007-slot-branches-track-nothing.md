# 0007: a slot branch tracks nothing

Status: accepted, 2026-09-08 (M2, task 17)

## The problem

`git::worktree_add` built a new slot with `git worktree add -b workspace-1 <path> origin/main`.
git sets an upstream for a branch created that way, so `workspace-1` tracked `origin/main`.

Two things follow, and they point the same way.

The first is what `git push` from the slot does. With `push.default` at its default, git
refuses a branch whose upstream has another name and tells the reader to say where to push;
with `push.default = upstream`, it pushes the slot's work to `main`. Neither is what a
workspace wants, and the second is the kind of surprise a multiplexer must not arrange.

The second is measurable. Setting the upstream is a write to `.git/config`, and git guards
that with a lock file. Two `workspace.create` calls on one project at the same time - one
press of the create key twice - then have one of them fail with

    error: could not lock config file .git/config: File exists

*after* it has already created the branch. The worktree is not built, the call fails in words
about a lock file, and a branch is left behind with nothing pointing at it. Measured on git
2.50: three of three concurrent pairs failed that way, and ten of ten pairs with `--no-track`
worked.

## The choice

`git worktree add --no-track -b <branch> <path> <base>`.

It also makes the two halves of `worktree_add` agree. The half that resets a branch that
already exists runs `git branch -f` and `git worktree add <path> <branch>`, which never set an
upstream, so before this a slot's tracking depended on whether its number had been used
before.

## What it costs

`git::is_dirty` asks `branch@{u}` first and falls back to `origin/<default branch>..branch`.
Every slot now takes the fallback, and the two answers differ in one case: with
`[worktrees] base` set to something other than the default branch, the fallback measures a
fresh slot against `origin/main` rather than against the base it came from, so it can call a
slot dirty that holds no work of its own. `is_dirty` is Task 18's gate in front of deleting a
workspace, so that error refuses a delete rather than allowing one, which is the safe
direction. The branch that reads `@{u}` is kept and tested, because a reader can still set an
upstream by hand.

## What was set aside

**Serialising the git work per project**, so two creates on one repository never overlap. It
would cover more than this one clash - `git branch -D` writes `.git/config` too, so Task 18's
delete could collide with a create the same way - but it is a lock and a map of locks to
reason about, and with `--no-track` neither call writes the file that was being locked. If a
later task finds a second place where git refuses concurrent work on one repository, that is
the point to build it rather than now.
