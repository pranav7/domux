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

## What it costs, and what Task 18 inherits

`git::is_dirty` asks `branch@{u}` first and falls back to `origin/<default branch>..branch`.
**Every slot now takes the fallback**, so the fallback is the normal path rather than the edge
case it was written as. `is_dirty_compares_against_the_default_branch_when_the_slot_has_no_upstream`
tests it as the normal path, and its fixture must not be given an upstream again: that would
take the main path back out of the suite.

The two ranges agree whenever `[worktrees] base` is `origin/<default branch>`, which is why
nothing else moved. They agree because the upstream `git worktree add -b` set pointed at the
**base**, never at the slot's own remote branch, so `@{u}..branch` and
`origin/<default branch>..branch` were the same range.

**They disagree when the base is anything else, and Task 18 meets this first.** Measured: a
fresh slot branched from `origin/release`, with an empty `git status`, answers `is_dirty =
true`, because `origin/main..workspace-1` holds release's own commit. So with `[worktrees] base`
set to something other than the default branch, **every workspace is born dirty**, and a gate
that refuses to delete a dirty workspace refuses all of them. That is the safe direction for a
destructive operation, and it is still wrong.

`a_slot_from_a_base_other_than_the_default_branch_reads_dirty_while_it_has_no_upstream` pins it,
as a limitation written down rather than a rule blessed. The fix is to give `is_dirty` the base
to compare against instead of guessing the default branch. It is not made here because
`is_dirty` has no callers yet: Task 18 builds them, it knows the base it is deleting against,
and the change cannot break anything that does not yet exist.

The branch that reads `@{u}` is kept and tested, because a reader can still set an upstream by
hand.

## What was set aside

**Serialising the git work per project**, so two creates on one repository never overlap. It
would cover more than this one clash - `git branch -D` writes `.git/config` too, so Task 18's
delete could collide with a create the same way - but it is a lock and a map of locks to
reason about, and with `--no-track` neither call writes the file that was being locked. If a
later task finds a second place where git refuses concurrent work on one repository, that is
the point to build it rather than now.
