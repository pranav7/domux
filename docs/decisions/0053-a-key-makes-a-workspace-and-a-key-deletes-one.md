# 0053: A key makes a workspace and a key deletes one

**Date:** 2026-09-16
**Status:** Accepted
**Decision:** `[keys.list]` binds `c` to `workspace.create` and `D` to `workspace.delete`.
Neither binding names a target, so each acts on the row under the cursor. `workspace.delete`
takes its workspace as one optional positional argument, and an absent one is read by
`Ctx::workspace_of_cursor`, which refuses a caller that has no cursor rather than falling back
to the caller's own workspace.

## Context

MUX-50 asks for the ability to make a workspace inside a project and to delete one, and notes
that V1 had both and that `worktree.conf` should still work.

Almost all of it was already here. M2 built `workspace.create` (task 17) and `workspace.clear`
and `workspace.delete` (task 18) on the job lane decision 0006 describes: the handlers, the
jobs, the confirmation copy that one builder writes for both surfaces, the overlay a key opens
through `ctx.from_key`, the CLI subcommands, and `worktree_conf.rs`, which is V1's three verbs
with containment rules V1 lacked and a rollback V1 lacked. What was missing was that no default
key pointed at any of it. The operations were reachable from a shell and from a `domux.toml` a
reader wrote themselves, and from nowhere a reader meets in normal use.

So this record is about two bindings and the one thing a binding needed that was not there.

## The keys

`c` unshifted and `D` shifted, which is the rule the table already followed: `n` renames a
workspace and is unshifted, `X` removes a project and is shifted. A key that takes something
away wears the shift.

The author was offered three choices on 2026-09-16: `c` and `D`; `c` and `x`, pairing with `X`
so that case says which row kind the key acts on; or V1's own `+` and `D`. The author chose `c`
and `D`.

- **`x` was set aside** because a slipped shift would move between two different destructive
  acts. Both ask first and the two questions name different things, so the risk is bounded, but
  it is a risk bought for nothing.
- **`+` was set aside** because it needs a shift on most layouts, and the leader table's own
  comment says a common operation should not need one. `c` is the initial of the verb.

`D` is also the key V1 used, so the muscle memory carries over.

## An absent target is not "the one I am in"

`workspace.delete`'s workspace was a required `String`, because a shell must name what it
deletes. A key carries no argument, so the field is now `Option<String>`, as
`workspace.clear`'s already was.

The two do not read an absent target the same way, and that is the point:

- **A clear** falls back to the caller's own workspace, through `resolve_workspace_param`. It
  puts a branch back, and the slot the caller is standing in is the one they mean.
- **A delete** goes through `Ctx::workspace_of_cursor`, which answers the row under the cursor
  when the keys are in a Projects box and refuses with `name a workspace to delete` anywhere
  else. A shell that typed `workspace delete` with nothing after it would otherwise remove the
  worktree and the branch of the slot it is standing in.

This is `project.remove`'s rule, held by `Ctx::project_of_cursor`, and the refusal is worth the
same words: a target a key carries is a target the reader can see, because the fill is on the
row it names. `domux workspace delete <ws>` still requires its argument, at the command line
where there is no cursor to read.

## Consequences

- The keys overlay has two more rows under `in a list`, drawn from `[keys.list]` like every
  other row there, so `?` lists them with no copy written for them. Four tests that assert on a
  help overlay that fits, or on a leader row being visible below the box block, were given two
  more rows of screen; each says so where it says why it is the height it is.
- The sidebar's hint row and the switcher's footer are unchanged. Both name two keys and then
  `?`, and the keys they name are the ones every row answers to; a box key that acts on one row
  belongs behind `?` with the rest.
- `workspace.clear` still has no key. MUX-50 did not ask for one, and it is the operation whose
  refusal depends on the job looking inside the slot.
- Nothing about `worktree.conf` changed. A slot made with `c` is made by the same handler and
  the same job as one made from a shell, so its setup runs the way it already did.
