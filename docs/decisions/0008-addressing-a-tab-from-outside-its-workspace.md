# 0008: addressing a tab from outside its workspace

Status: accepted, 2026-09-09 (M2, task 23)

## The problem

A tab could only be reached by a caller already looking at it.

`Ctx::resolve_tab_param` resolves a `tab` parameter with
`Model::resolve_tab(&view.workspace, target)`, which searches one workspace: the calling
client's. Every method that takes a tab goes through it, so `pane.list`, `tab.rename`,
`tab.clear_name`, `tab.close` and `tab.select` all answer `not_found` for a tab in any other
workspace, whatever the caller passes.

That is invisible while every caller is a key press, because a key comes from a client that
is by definition in the workspace it is acting on. `import v1` is the first caller that is
not: it runs in a shell with no pane, creates workspaces nobody is looking at, and has to
put tabs in them. Two of its calls could not work, for one cause.

`resolve_pane` has never had this shape. It takes a bare pane id and searches the whole
model, because a pane id is unique. **A tab id is equally unique, so the difference between
the two is an inconsistency rather than a safety property.**

## The choice

Two additions, neither of which changes an existing behaviour.

`TabInfo` gains `cwd`, the directory of the tab's focused pane. Before it, a tab's directory
could only be read through `pane.list`, so nothing could ask where a tab was unless it was
already looking at it. `tab.list` takes a workspace and has never been view-scoped, so the
field makes the answer reachable without touching any resolver.

`TabCreateParams` gains `name`, applied inside `tab::create` through the same
`Model::rename_tab` that `tab.rename` calls. One implementation, two callers. It is also the
honest shape for the domain rather than a way around the resolver: in V1's session file a
window and its name are **one fact**, and V2 splitting them into a create and a rename is
V2's artefact, not something the caller meant.

## What was deferred, and why

**The general fix is to let a bare tab id resolve globally, as `resolve_pane` already does.**
It is the more correct change and it is not being made yet.

`resolve_tab_param` is shared by five methods, so widening it enables a state nobody has
designed: `tab.select` pointed at a tab in another workspace. What a client should do when
asked to select a tab it is not near - move, refuse, or select and move the view with it - is
a question for whoever owns tab selection, and enabling it by accident in three handlers at
once, for inputs that today error cleanly, is how a defect gets found a milestone later
instead of here.

So `tab.rename` stays view-scoped. A caller with no view names a tab at creation instead.
**When `tab.select` has a designed answer, the resolver should be widened and
`TabCreateParams::name` re-examined** - it stays useful on its own merits, but it stops being
the only way to name a tab from outside.

## What it did not fix

Neither `tab.rename` nor `tab.create` refuses a name that reads as a tab number.
`Model::resolve_tab` runs its number pass first, so a tab named `2` can never be resolved by
that name for the life of the tab. This is the tab-shaped twin of the defect decision 0007's
milestone fixed for workspaces with `WorkspaceHandle::reads_as_handle`, where the cost was a
removed worktree; here it is a wrong tab.

`tab.create` was deliberately left to match `tab.rename` rather than guarding one of the two
paths, because two ways in that disagree about what is legal is worse than either answer.
Fixing it is a change to `tab.rename`'s contract. `import v1` makes it easier to reach, since
V1's window names come from the author's own sessions and nothing has ever stopped one being
`2`.
