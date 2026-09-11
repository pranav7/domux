# 0018: Hooks on an exited record

**Date:** 2026-09-10
**Status:** Retired by decision record 0030, which removed the exited record this is entirely about. A session that ends leaves no record for a late hook to land on, and only `SessionStart` makes one. Kept as the record of why the exited record behaved as it did.
**Decision:** A hook event on a record whose session is over changes nothing, down to its
last activity time. `SessionStart` is the exception: when its session id matches an exited
record, that record comes back rather than a second one starting.

## Context

The architecture spec's state machine table gives a row for every live state and no row for
`exited` as a starting state, so both halves were open.

The first half is a race. A session ends and the agent sends `SessionEnd`; a `Stop` from the
turn that was finishing arrives after it. Nothing orders the two, and the pane is still there,
so the late `Stop` lands on a record that has already exited.

The second half is what resume needs. Resume types the kind's relaunch line into the pane, and
that line carries the session id the exited record holds. The new session's first hook is a
`SessionStart` with that same id. If it started a second record, the list would keep an exited
row for a session that is running again, which only a dismiss, a workspace clear or a project
removal would take away, and the recap and the session name the old record carried would not
follow the session that actually continued.

## The rule, in two places

`transition` names the ignored events, so the table test covers them:

```rust
(Exited, UserPromptSubmit | PreToolUse | PostToolUse | Notification
       | PreCompact | PostCompact | Stop) => Exited,
```

`SessionEnd` and `ProcessGone` on an exited record also answer `Exited`, through the arm that
answers `Exited` for every state. So every hook but `SessionStart` leaves the state where it
is, and `(_, SessionStart) => Idle` is the one way back.

`Model::report_agent` acts on that. When the state it came from and the state it moves to are
both `Exited`, it returns the outcome before it writes anything: not the pane, not the session
id, not the transcript path, not the reason, not `last_activity_at`. The row keeps saying
`exited 12 min ago` and keeps counting from the real exit.

`api::agent::report` reads the outcome's `from` for the same reason. It re-reads the
transcript on `SessionStart`, `UserPromptSubmit` and `Stop`, and a handler that discarded
`from` re-read it for a late `Stop` too, rewriting the recap and the session name of a dead
record. That was a real defect, fixed in the commit that added
`a_hook_that_arrives_after_the_session_ended_leaves_the_exited_record_alone`.

## How a resumed session finds its record

`report_agent` looks for a record by kind and session id first, and only then for the live
record on the pane. A resumed session matches on the id, so it lands on its own record
whatever else is on that pane.

The observer will usually have put a placeholder there in the meantime: the relaunch line
starts a process, the next tick sees a known agent with no record, and an `unknown` record
appears. When the `SessionStart` arrives, the placeholder is removed rather than exited,
because it holds no session id and so names no session that could be lost with it. It leaves
as `Event::AgentDismissed`, and the resumed record takes the pane back. Its id is retired
rather than reissued, the way every removed id is.

A pane that already holds a *different* live session is the other case, and there the old
record exits: one pane hosts at most one live agent.

## Consequences

- A late hook is silent. Nothing in the row moves, and no event is published.
- A resumed session keeps its session id, its recap, its session name and its place in the
  list. `a_resumed_record_stays_exited_until_a_hook_says_otherwise` pins that resume itself
  does not change the state: typing a line into a pane is not evidence that a session started,
  and the record leaves `exited` only when the new session's first hook says so.
- `SessionStart` re-reads the transcript, which is why a resumed row shows its recap and its
  name at once instead of after its first turn.
  `a_session_start_alone_reads_the_recap_and_the_name_a_resumed_session_already_has` is the
  only test that pins it.
- Nothing but `SessionStart` moves an exited record, so a record that exited when it should
  not have would be corrected by a new session or by a dismiss, not by a later hook. That is
  the cost of this rule, and what carries it is the narrowness of the exits: a record exits
  only when its own process id goes away or its pane goes away (decision record 0017), and
  `SessionEnd` is the agent saying so itself.
- The observer's way back is separate and does not use this path.
  `(Exited, Observed) => Unknown` says a new process is on the pane, and the observer turns
  that into a new record, leaving the exited one listed with its recap.
