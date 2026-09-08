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

## What it cost, and what was done about it

`git::is_dirty` asks `branch@{u}` first and falls back to a comparison of its own. **Every slot
now takes that fallback**, so it is the normal path rather than the edge case it was written as.
`is_dirty_compares_against_the_base_when_the_slot_has_no_upstream` tests it as the primary path,
and its fixture must not be given an upstream again: that would take the main path back out of
the suite.

The fallback used to guess `origin/<default branch>`, and the guess was invisible for a reason
worth writing down: the upstream `git worktree add -b` set pointed at the **base**, never at the
slot's own remote branch, so `@{u}..branch` and `origin/<default branch>..branch` were the same
range whenever the base *was* the default branch. Dropping the upstream separated two ranges
that had silently coincided.

Measured, before the fix: a fresh slot branched from `origin/release`, with an empty
`git status`, answered `is_dirty = true`, because `origin/main..workspace-1` holds release's own
commit. With `[worktrees] base` set to anything but the default branch, **every workspace was
born dirty**, and Task 18's gate would have refused to delete any of them.

So `is_dirty` takes the base rather than guessing it: `is_dirty(path, branch, base)`. The caller
already knows it - `git::base_ref` is what produced the base the slot was made from - and the
comparison base is a fact about how the workspace was made, not a choice about what "dirty"
should mean. Passing it forecloses none of Task 18's options: it serves "differs from its base",
"has uncommitted changes", or both.
`a_slot_from_a_base_other_than_the_default_branch_reads_clean_until_it_holds_work` pins it, with
a second slot on the default base so both bases are visibly answered rather than one asserted
alone.

The branch that reads `@{u}` is kept and tested. It is the better answer when a slot has an
upstream, because work the reader pushed is not work a delete would lose.

**What Task 18 still inherits.** A slot whose work is pushed to `origin/workspace-1` without
`-u` has no upstream, so it is measured against its base and reads dirty even though the commits
are safely on the remote. That is unchanged by this decision - it read dirty under the old
tracking too, for the same reason - and what to do about it belongs with the gate that acts on
the answer.

## What was set aside

**Leaving `is_dirty` guessing and documenting the divergence as a limitation.** That was the
first answer, and it was wrong: a gate that refuses to delete any workspace is the failure where
a reader stops trusting the tool rather than the gate, and it would have reached Task 18 looking
like Task 18's own defect. The signature change is free while `is_dirty` has no callers.

**Serialising the git work per project**, so two creates on one repository never overlap. It
would cover more than this one clash - `git branch -D` writes `.git/config` too, so Task 18's
delete could collide with a create the same way - but it is a lock and a map of locks to
reason about, and with `--no-track` neither call writes the file that was being locked. If a
later task finds a second place where git refuses concurrent work on one repository, that is
the point to build it rather than now.
