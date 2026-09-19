# 0057: A turn that dies on an API error ends

**Date:** 2026-09-18
**Status:** Accepted. Adds one event to the nine the installer writes for Claude.
**Decision:** `StopFailure` is the same event as `Stop`. The installer writes it, and
`hooks::parse_claude` reads both as `AgentEvent::Stop`, so a turn that died leaves the row idle
rather than working. The error it carries is not a reason: a reason is what a waiting row says,
and a row whose turn died is not waiting on you.

## Context

The author's Navigator showed a row working while its agent sat at an empty prompt. The record
was `chat-partner-issues`, working since 13:11:44, and the pane had said nothing since 13:11.

What the evidence said, in order:

- The transcript's last entry was a prompt typed at 13:11:44.599, and `last_activity_at` was
  13:11:44.609. So the last hook that landed was `UserPromptSubmit`, which is a working event.
- Nothing followed it: no assistant entry, no `[Request interrupted by user]` marker, and no
  render of the prompt on the screen or in the pane's scrollback.
- The agent's own process was alive, so nothing ended the record, and `working` is left only by
  `Stop`, `Notification`, `PreCompact`, `SessionEnd` or the process going away.

Reading Claude Code 2.1.276 for how else a turn can end found `StopFailure`, an event domux did
not install. Its main loop returns through `executeStopFailureHooks` on an API error and never
reaches the `Stop` hooks, and the payload names which error: `rate_limit`, `overloaded`,
`server_error`, `invalid_request`, `max_output_tokens`, `authentication_failed`,
`billing_error` and the rest of that enum.

So a turn that dies on any of those ended with a hook domux installed no line for. The row kept
the working word, the turning star and the band for the rest of the session, and the sidebar and
the switcher said an agent was busy that was waiting at a prompt.

## The rule

There is one thing to know about a turn that died, which is that it is over. `Stop` already
means that, the state machine already has the row it needs, and `transition` is unchanged: no
new state, no new event, no new column in the table. `StopFailure` is one more spelling of an
event domux already reads, the way `session.idle` and `session.error` are for OpenCode.

The error itself is dropped. `reason` is the sentence a waiting row shows, put there by a
notification that stops for you (decision record 0037), and an API error stops for nobody: it
has already happened, and the agent is back at its prompt. Showing `overloaded` in the reason
slot would put a row in front of the author that reads like an agent asking a question.

## What this leaves

- **A turn cancelled before its request goes out still leaves the row working.** Claude Code
  writes the prompt to the transcript and fires `UserPromptSubmit` first, and cancelling in that
  window fires nothing further and writes no interrupt marker. There is no such event to install:
  the list of hook events in 2.1.276 has no cancel. This is the case the author met, and this
  record does not answer it.
- **A lost hook is still not made good later.** A `Stop` or a `StopFailure` that never reaches
  the socket leaves the row working until the process dies, which is decision record 0019's
  behaviour and the reason a row stuck on working is worth going to look at.
- **An install is what carries this.** The events live in the settings file the installer wrote,
  so a machine gets the new line by running `domux install claude --apply` again. The install is
  the repair: it takes every line the previous binary wrote and writes one line per event, so a
  file that never had `StopFailure` gets it.
- **Codex and OpenCode are unchanged.** Neither has an event of this shape.

## Alternatives

- **A state of its own, such as `failed`.** It is a new row in the table and a new thing for
  every surface to draw, to say something the author reads once and cannot act on. The agent is
  idle; the transcript says why.
- **`waiting` rather than `idle`, so the row keeps a dot.** A dot means an agent is blocked on
  you (decision record 0030), and this one is not: nothing is waiting for an answer, and the dot
  would stand until the next turn.
- **A timeout: a row that has said nothing for some minutes falls back to `unknown`.** It would
  cover the cancelled turn as well, which is what the author actually met, and it is the one
  thing here that guesses. `unknown` is honest about a row nothing reports on, so this is worth
  its own record rather than a line in this one.
- **Read the transcript for the end of a turn.** A cancelled turn and a turn still thinking look
  the same in the file: a user entry and no reply yet. The tick would have to wait a while before
  deciding, which is the timeout above wearing a transcript.
