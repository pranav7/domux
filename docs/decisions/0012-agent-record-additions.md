# 0012: Four fields the agent record adds to the spec

**Date:** 2026-09-10
**Status:** Accepted
**Decision:** `domux_core::model::agent::Agent` carries `workspace`, `last_pane`,
`transcript_path` and `source` beyond the architecture spec's 3.2 record. Each answers a
question an exited record cannot answer from the pane id alone.

## Context

The spec's record stores an agent's location as its pane id. That is enough while the session
runs and answers nothing once it ends: an exited record has no pane. `exit_record` takes the
pane and the process id away, because a record that still named a pane would claim a place it
no longer holds, and `live_agent_on_pane` would find a dead session sitting on a live pane.

So the facts the row and resume still need after an exit have to be stored, not derived.

## The four

**`workspace: WorkspaceId`.** The place line (`project › workspace › tab`) and resume both
need the workspace, and `agent.list --workspace`, the clear rule and the delete rule narrow by
it. It is set when the record is created and rewritten when a report lands on a pane the
record was not on, so it follows a session that moved rather than naming where the session
started.

**`last_pane: Option<PaneId>`.** The pane the record last ran in. `place` falls back to it
when `pane` is `None`, which is what puts the tab on an exited row, and resume types the
relaunch line into it ("in its original pane", architecture spec section 5). It is written
alongside `pane` and never cleared, so an exit keeps it.

**`transcript_path: Option<PathBuf>`.** The hook payload gives it and the recap reader reads
its tail for the recap and the session name. It is not derivable in V2: the spec dropped V1's
directory encoding of the transcript path, which is the point of the hook carrying it.

**`source: AgentSource`, one of `Hook`, `Observer`, `Restore`.** Where the record came from. It
travels on `Event::AgentCreated`, so a subscriber can tell a session that reported itself from
a placeholder the observer made out of a process name, and M4's `doctor` counts the
placeholders to say which kinds have their hooks installed. `Restore` is not a creator:
`mark_agents_exited_on_restore` stamps it on every record that was live when the server last
wrote `state.json`, alongside marking it exited, so a row that says `exited` because the
server restarted is told apart from one whose session ended.

## Consequences

- `last_pane` and `transcript_path` carry `#[serde(default)]`, so a state file written without
  them still restores. `source` does not, and does not need to: `agents` first appears at
  schema version 3, and `state_file::v2_to_v3` inserts an empty array, so no state file has an
  agent without a source. `pid` is `#[serde(skip)]` and comes back as `None`, which is why a
  restore marks every record exited rather than claiming a process it cannot see.
- Four more fields to keep true. Each is written by `Agent::new` and by exactly one other
  place, and every one of those places is in `Model`: `workspace`, `last_pane` and
  `transcript_path` by `report_agent`, `source` by `mark_agents_exited_on_restore`.
- `agent.list`, `agent.get` and `agent.self` answer with `AgentInfo`, which is built from these
  fields rather than exposing the record, so a later field is additive on the wire.
- The record is what `state.json` persists, so adding a field is a schema change and removing
  one is a migration. `last_message` is on the record and written by nothing in M3 (interface
  spec open question 15); it stays rather than being removed and added back.
