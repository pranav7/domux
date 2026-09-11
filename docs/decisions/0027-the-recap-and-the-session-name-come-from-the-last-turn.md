# 0027: The recap is the last turn's, and the session name is the checkpoint's

**Date:** 2026-09-10
**Status:** The name half stands. **The recap half is replaced by decision record 0034**, which
takes the fallbacks away, drops the rule about which turn a summary belongs to, and moves the
reading off the hooks and onto the tick. Read 0034 for what the reader does now; what is below
about the recap is the state MUX-28 was raised against. Amends M3 plan assumption 6 and closes
the M3 open item "the `/rename` shape is unverified against real usage".
**Decision:** A summary counts as the recap only when it was written after the last prompt;
otherwise the recap is the last thing the agent said in words. The session name is the last
`custom-title` entry, then `agent-name`, then the `/rename` command's arguments. The transcript
is re-read on `Notification` and the two compact events as well as on `Stop`,
`UserPromptSubmit` and `SessionStart`.

## Context

Two reports against `agents::recap`, and both come down to the same thing: M3's reader was
written against a transcript shape that has moved.

**MUX-19**, "agent name in agent navigator to match the agent name in the session": the reader
typed `/rename per-line-extraction-tod`, Claude Code answered "Session renamed to:
per-line-extraction-tod", and the row went on saying `claude`.

**MUX-20**, "agent recap and notifications not working correctly": a row whose recap described
work from a good while earlier.

## The name

M3 read the name as the last `<command-name>/rename</command-name>` entry's `<command-args>`,
and the M3 plan recorded that the shape was specified rather than observed: a search of 152
transcripts found no real `/rename`. That is still true, and it is why the row never changed.

What Claude Code actually writes is a `custom-title` entry, restated every time it checkpoints
the session, and an `agent-name` entry carrying the same string beside it. Restated is the
useful half: a rename made early in a long session is written again near the end of the file,
where the tail read always reaches it. The `/rename` entry is written once, at the moment of the
rename, so on a transcript over `FULL_SCAN_BYTES` an early rename fell outside the window even
when the entry was there.

So three sources, read in that order. The `/rename` parse stays last rather than being deleted:
it costs one branch, it is what older transcripts carry, and a source that is sometimes right is
worth more than a name that is absent.

An entry whose title is empty or only spaces leaves the name it found alone. One of these
entries is written per checkpoint, so a blank one is a line the reader will meet, and blanking a
name on it would flicker the row back to `claude`.

`agentName` is also a field on ordinary messages in a session run by a named teammate. The
hyphen in the type word is what keeps those lines out of the parser, and the cheap substring
filter matches on the hyphenated form for that reason.

## The recap

M3 preferred the freshest `away_summary` or `/recap` output and fell back to the last
`ai-title`. An `away_summary` is Claude Code's periodic summary of the whole session, written
now and then rather than per turn, so the freshest one in a long session can describe work from
several turns back. M3 showed it anyway, and a row that had just finished something said what it
had been doing half an hour earlier.

The rule now is that a summary has to belong to the turn the transcript ends on.
`recap::last_turn` walks back from the end of the file to the prompt that started the last turn.
If it meets the summary first, the summary is this turn's and it wins, because it is the better
written line. If it meets the prompt first, the recap is the last thing the agent said in words
- its own last text, first sentence - which is the last turn by definition.

Three fallbacks under that, in order: a stale summary where the agent has not answered in words
yet, because a stale line says more than a blank one; then the `ai-title`; then nothing.

**Backwards, and stopping at the prompt**, because everything it asks is at the end of the file:
the answer is a handful of entries however long the session is. The forward pass cannot answer
either question - "is this the newest summary" is a forward question and "was it written after
the last prompt" is not - and answering the second one forwards would mean parsing every user
entry in the file, which is what the cheap substring filter exists to avoid. It gives up after
`TURN_LINES` and says the summary is not this turn's, which is the cautious half of the rule.

A `user` entry whose message content is a list of blocks is a tool result, not a prompt, and does
not end the walk. A slash command is text but it is not a turn, and does not end it either. A
`/recap` writes its line as the stdout of a local command, and walking back the output turns up
before the command that produced it, so the output only counts once `/recap` itself appears
under it. An entry marked `isSidechain` is a subagent's, and what a subagent said is not what
this session said.

## When the transcript is re-read

M3 read it on `Stop`, `UserPromptSubmit` and `SessionStart`. `Notification` joins them, and it
is the one that matters: an agent that has stopped to ask you something is the row you read
hardest, and M3 left it showing whatever the last `Stop` had found. `PreCompact` and `PostCompact`
join too - a compaction is a gap in the conversation and the recap on the far side of it is worth
re-reading - and they cost nothing, being rare.

`PreToolUse` and `PostToolUse` stay out. They fire many times a turn over a transcript the agent
is appending to, so the modification time has always moved and every one of them would read the
file whole; an 8 MB transcript would be read dozens of times a turn. The row is turning a glyph
while they arrive, which already says the recap is a turn behind.

## What this does not do

**It does not read `~/.claude/sessions/<pid>.json`**, which holds a live `name`, `nameSource` and
`status` per running session and would answer the name question directly. It would be a second
source of truth beside the transcript, keyed on a process id rather than on the
`transcript_path` the hook hands over, and it exists for one of the three kinds. The transcript
is what the manifest declares (`RecapSource::ClaudeTranscript`) and it is where both fields
already come from.
