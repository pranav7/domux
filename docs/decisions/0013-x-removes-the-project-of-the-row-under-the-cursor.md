# 0013: `X` removes the project of the row under the cursor

**Date:** 2026-09-09
**Status:** Accepted
**Decision:** `[keys.list]` binds `X` to `project.remove` with no project named. A
`project.remove` that names none acts on the project of the row under the cursor while the
keys are in a Projects box, and is refused everywhere else.

## Context

MUX-9 asks for a shortcut to remove a project from the sidebar, over a screenshot of a stray
project that had to be removed with a shell command. Interface spec 7.3 already sketched `X`
in the box for this, and the M2 plan's closing note said the handler existed and binding it
was one line in `[keys.list]`.

It was not one line, because the handler had no target a key could give it.
`ProjectRemoveParams::from_args` required a project as argument 0, so a binding with no
argument failed to parse, and the handler refused a call that named none.

The refusal was deliberate: "a removal that names nothing and asks for nothing is a mistake,
not remove everything". That reasoning holds for a shell and not for a key. A key in the box
has a target the shell does not, the row the fill is on, and the reader can see which one it
is. So the fallback is `Ctx::project_of_cursor`, which asks `list::in_a_box` and keeps the old
refusal, word for word, for every caller that is not in one. `Ctx::project_of_view`, which
`workspace.create` uses, would have fallen back to the caller's own project and turned
`domux2 project remove` with nothing after it into a removal.

## Consequences

- `X` works in the switcher too, because the switcher's box is the sidebar's box and both read
  one `[keys.list]` table. That is the first key to open a confirmation over the switcher,
  which is the case `ClientView::push_overlay` and `pop_overlay` were built for.
- The key asks before it acts, on the screen, as `project.remove` from a key already did.
  Nothing on disk is touched, and the confirmation says so.
- `reseat_stranded_clients` now also clears a `projects_cursor` naming a workspace that has
  gone, so the box does not come back from a removal with no fill in it.
- `X` is not bound in `[keys.bindings]`, so it reaches nothing from a pane. `c`, `+`, `D` and
  `A` from interface spec 7.3's sketch are still unbound.
