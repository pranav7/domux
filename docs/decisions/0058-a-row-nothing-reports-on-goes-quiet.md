# 0058: A row nothing reports on goes quiet

**Date:** 2026-09-18
**Status:** Accepted. Narrows decision record 0019, which left a row on `working` until its
process died. Follows 0057, which handles the one turn end that does send a hook.
**Decision:** A record that says `working` or `compacting`, and whose transcript has not been
written for ten minutes, moves to `unknown`. `AgentEvent::Quiet` is that event, a column in
`transition`'s table like any other, and `agents::quiet::poll` sends it from the core's tick.
Nothing else changes about the record: it keeps its pane, its recap, its name and its
`last_activity_at`, it does not go `unseen`, and the next hook takes it back.

## Context

The author's Navigator showed `chat-partner-issues` working while the agent sat at an empty
prompt, and it had said so for 35 minutes. The last hook was a `UserPromptSubmit` at 13:11:44,
ten milliseconds after Claude Code wrote the prompt to the transcript. Nothing followed it: no
assistant entry, no `[Request interrupted by user]` marker, no render of the prompt on the screen
or in the pane's scrollback, and the agent's own process was alive, so nothing ended the record.

The turn was cancelled in the window between the prompt being written and its request going out.
Claude Code fires `UserPromptSubmit` in that window and nothing afterwards, and 2.1.276 has no
hook event for a cancel, so there is no line to install. 0057 covers the other end, a turn that
dies on an API error, because that one does send `StopFailure`.

`working` and `compacting` are the two states only a hook can leave. An agent has more ways to
stop than it has hooks to say so, and each one leaves the row holding the working word, the
turning star and the band, which is domux saying an agent is busy when it is not. 0019 accepted
that: a lost hook is not made good later, and a row stuck on working is the author's cue to go
and look. This record keeps the first half and drops the second. A row that lies is worse than a
row that admits it does not know.

## The rule

`unknown` already means an agent is running and nothing is reporting, which is exactly what has
been established. So this invents no state and guesses nothing about what the agent is doing: it
takes a state away. Neither `idle` nor `waiting` would do. Nothing said the turn ended, so the
row is not idle, and nothing is blocked on the author, so the row has no dot to draw (0030).

The evidence is the transcript's modification time, not the last hook. Every hook a session sends
comes with entries in that file, and an agent working writes to it continuously, so a transcript
standing still is the session standing still. It costs one `stat` per busy record per tick, beside
the read `agents::recap` already does, and it needs no state of its own: the file's own time is
the record of when the session last did something.

**Ten minutes is measured, not chosen.** Across four of the author's own transcripts, 33,500
entries and 890 silences inside a turn, the longest a transcript went untouched while its agent
was genuinely working was 191 seconds, and the rest of the distribution ends between two and
three minutes. Ten minutes is about three times the longest real one. The cost of being wrong in
the short direction is a row going quiet while its agent works, which is the thing this was
built to stop, so the threshold is generous on purpose.

## What this leaves

- **A row waits ten minutes to tell the truth.** Nothing here makes it faster, because nothing
  faster is evidence. A row stuck on working is still worth going to look at, and now it stops
  lying on its own.
- **`waiting` still stands for as long as it takes.** A waiting row is blocked on the author and
  says so until it is answered, which is what its dot is for. A quiet period cannot tell a
  question waiting an hour from a question nobody will answer, so it does not try.
- **A record with no transcript never goes quiet.** OpenCode's plugin sends no path, and a record
  whose hooks never carried one has nothing to stat. A file domux cannot read, and one whose time
  is ahead of this machine's, are no evidence either, so both leave the row as it is.
- **A hook and a transcript write arrive together, and the rule leans on that.** A hook that set
  `working` while the transcript stood still would be undone by the next tick. In Claude Code the
  prompt entry is written before the hook runs, measured at ten milliseconds apart, and every
  tool event writes entries too, so the case is theoretical. If it ever happens the row reads
  `unknown`, which is still the honest answer for a session whose file is dead.
- **Nothing is persisted or handed over.** The state is in the record, which the handover already
  carries, and the timer is the file's own modification time, so `HANDOFF_FORMAT` is unchanged
  and a server that has just taken over decides on the same evidence as one that has been up for
  hours.

## Alternatives

- **Read the last hook's time rather than the file's.** `last_activity_at` is already on the
  record, so it looks free. It is not evidence: a turn that writes a long answer and calls no
  tool sends no hook for as long as it takes, while the transcript grows the whole time.
- **Keep both, and go quiet only when neither has moved.** It is the strongest signal and it
  costs a second clock: `last_activity_at` is written from the injected clock, so a test with a
  fixed clock could never age it, and the interface test that proves this behaviour could not be
  written. The file's time is enough for every case the author has met.
- **Watch the pane's screen, or its foreground process.** Rejected before, twice, and for the
  reason that still holds: an agent inside an editor's terminal is never in front, and an agent
  running a tool puts that tool in front (0045, 0055). A record is not read off a screen.
- **A shorter threshold, two or three minutes.** It is inside the measured distribution of real
  silences, so it would blank rows that are working. Ten minutes costs a longer lie in exchange
  for never telling a new one.
