# 0017: The observer runs inside the core's tick

**Date:** 2026-09-10
**Status:** Accepted
**Decision:** The observer is a set of plain functions the core calls, not a task of its own.
`agents::observer::run` runs from the once-a-second tick the core already runs for pane
titles; `agents::observer::pane_gone` runs from the two places a pane leaves; `any_working`
is a read the animation timer asks for (decision record 0019).

## Context

The architecture spec calls the observer a task. The core already walks every pane once a
second and asks `ProcessInspector` for the foreground process, for the pane title and for
passthrough. A second task would ask the same question again, need its own copy of the pane
list, and have to send its answers back to the core, which owns all mutable state.

## Consequences

One walk per second, and one owner of the Model. The core collects one `PaneProcess` per
pane from the inspector answer it already reads for the pane boxes, then calls `run` once
with the model, that list, the manifest registry, the inspector and the time. `run` answers
with the events it produced, which the core publishes the way it publishes every other
change. Tests call `run` and `pane_gone` directly, with a fake inspector holding a per-pane
process table, so the observer's rules are tested without a clock.

The observer cannot run at a different interval from the tick. If it ever needs one, it
becomes a task that sends `CoreMsg::Observed(Vec<PaneProcess>)` and the rules move with it
unchanged.

### A pane leaves by two routes, so `pane_gone` has two call sites

The exit hook is not in `Core::close_pane`. `api::pane::close` never calls it: it calls
`Model::close_pane` itself and pushes the panes onto `Ctx::pending_kills`, and
`api::tab::close`, `api::project::remove` and `Core::workspace_deleted` do the same. All of
them funnel through `apply_side_effects`'s kill loop, so that loop is one call site and the
`CoreMsg::PaneExited` arm is the other. A hook in `close_pane` alone would leave an agent
live and reading `working` after the user closed its pane.

The two are told apart only by a pane configured with `remain_on_exit = true`. With the
default, `Core::after_batch` closes an exited pane inside the same batch as the exit, so the
kill loop ends the record and a test cannot see which mechanism did it.
`a_pane_whose_child_exits_marks_its_agent_exited_while_the_pane_stays` is the test that can.

### The cost of living inside the core

`run` takes its pane list from its caller and hands each pane to `Model::observe_agent`,
which panics on a pane the model does not hold. Nothing in `run` checks. The invariant holds
today because the core is the only caller and builds that list from the model's own panes,
and because a pane leaves the model and its runtime in the same batch of a single-threaded
core. If a later caller breaks it, the server dies inside a once-a-second timer rather than
returning an error. One `continue` in `run` buys that back, and M4 adds callers.
