# 0037: A notification waits only when it stops for you

**Date:** 2026-09-11
**Status:** Accepted. Amends decision 0030's dot rule at its source, the Claude adapter.
**Decision:** The Claude adapter reads `notification_type`. Five types make the waiting event:
`permission_prompt`, `idle_prompt`, `elicitation_dialog`, `elicitation_url_dialog` and
`agent_needs_input`. Every other type, and any type a later release adds, changes nothing. A
payload with no type waits.

## Context

The dot is the one visual affordance domux ships for an agent that needs you. M4's Notifier,
the toast, the desktop notification and the sound, is out of scope by the author's ruling of
2026-09-11, and a dot is drawn only while an agent is waiting (decision record 0030). So what
turns `waiting` on is what the dot means.

The state machine takes every Claude `Notification` hook to `waiting`. When M3 wrote the
adapter the fixtures were specified rather than recorded, and Claude Code's notification
carried two types: a permission prompt and an idle prompt. On 2026-09-11 it carries twelve:
authentication success, four elicitation steps, two subagent events and three quota
auto-resume events. `auth_success` says a login completed. `agent_completed` says a subagent
finished while its parent keeps working. A dot for either says "waiting on you" about an agent
that is not.

## The rule

A notification is the waiting event when Claude has stopped and cannot go on without you: it
asked permission, its input sat idle, an MCP server asked you something, or a subagent did.
The adapter lists those five. Anything else parses to no event, and `agent.report` answers
without touching the record, which is what it does for any hook event domux does not track.

A payload with no `notification_type` waits, because the Claude Code that omitted the field
sent a notification for the two waiting cases and nothing else.

`idle_prompt` stays. It fires once the input has sat idle for a while after a turn, which is
Claude saying it is waiting for you. So a session that finished a turn and heard nothing draws
the dot after that interval rather than at `Stop`. Whether the dot belongs at `Stop` instead
is a separate question, and this record does not decide it.

## Consequences

- The notification fixture carries `notification_type`, as the recorded payload does.
- A notification type Claude Code adds later draws no dot until someone lists it here, and
  never draws a wrong one.
- Codex and OpenCode are unchanged. Their waiting event is their permission request, and
  neither sends an idle notification, so a finished Codex or OpenCode turn never draws the dot.
