# 0034: A recap is an entry the agent wrote, and the tick reads it

**Date:** 2026-09-11
**Status:** Accepted. Replaces the recap half of decision record 0027 and closes the bug half of
MUX-28. The name half of 0027 stands unchanged.
**Decision:** The recap is an entry the agent wrote as a recap, and nothing else. The last one
stands until the agent writes another. The core's once-a-second tick reads it, no hook does, and
a read takes only the bytes the agent has appended since the last one.

## Context

MUX-28 came with a screenshot of two rows and two complaints.

One row said "Now let me verify visually with screenshots before publishing", and that session
had never written a recap. The other showed a recap the pane under it had already replaced.

Both are the same reader, and a count of real transcripts explains both. Of 37 recent sessions,
2 carry a recap entry at all. Of the 17 with five turns or more, 3 do, and the two long ones
average one recap per six turns. So nearly every recap the Navigator has ever drawn came from a
fallback rather than from a recap, and the one case that did have recaps showed them late.

## A recap is an entry the agent wrote

Claude Code writes one as an `away_summary`, and `/recap` writes one as the output of a local
command. The reader takes those two and stops there.

0027 read three more things in a ladder under them: the last thing the agent said in words, then
a stale summary, then the `ai-title`. The words are what put a sentence from the middle of a turn
in the slot a recap belongs in, and the title names the whole conversation rather than anything
the agent did. Both were shown where a reader expects the agent's own account, and CLAUDE.md
already said what should happen instead: a recap that did not arrive is absent.

The fallbacks were also the only thing the "does this summary belong to the last turn" rule
existed to arbitrate. With the words gone the rule decides nothing, so the backwards walk over
the end of the file goes with them, and `last_turn`, `assistant_text` and `TURN_LINES` go with
it.

## The last recap stands

When the newest recap predates the last prompt, the row keeps it rather than blanking.

At one recap per six turns, a rule that cleared the row at each new prompt would empty it almost
as fast as it filled it, and the reader would spend most of its life showing nothing in a session
that does produce recaps. So the recap behaves the way the session name already does: the agent
sets it, and it stands until the agent sets another. The state beside it says the rest, because a
row that is working says so with its glyph and its word.

## The tick reads it, and no hook does

The recap does not arrive with the hook that ends the turn. Claude Code writes the entry minutes
afterwards: in the session that was measured, a stop hook at 12:16 and the recap for that turn at
12:19, with the next stop hook at 12:22. So 0027's read on `Stop` found the file without it, and
the next `Stop` found the one before. That is the whole of "recaps don't match", and no set of
hook events fixes it, because the writer is not on the hook's schedule.

The core's tick asks once a second instead. One rule in one place: the six events 0027 re-read on
are gone from `agent.report`, which now reads no transcript at all.

## Reading only what was appended

Polling a file the agent is writing to cannot mean reading it whole. That is exactly why 0027
kept `PreToolUse` and `PostToolUse` out: at a full read each time, an 8 MB transcript would have
been read dozens of times a turn, and a poll would be worse.

A transcript is appended to and never rewritten, so the bytes behind a cursor cannot change and
re-reading them would answer what they answered before. The reader keeps a cursor per file and a
few running fields. A poll is one `stat` while the file sits still, and a few kilobytes while the
agent writes, whatever the file has grown to. Dropping the `ai-title` is what makes this work: it
sits at the head of the file and it was the only reason to read the head at all, so the first
read of a long transcript now starts `TAIL_BYTES` from the end and never looks further back.

Two things can go wrong with a cursor, and both are cheap to answer. A file shorter than the
cursor is not the file the cursor was counting, so the reading starts again. A file that was
replaced by a longer one would otherwise be read from the middle of a line forever, so the reader
checks that the byte behind its cursor is still the newline it stopped on, and starts again when
it is not. Neither happens while Claude Code is appending, and the reader does not depend on that
being true.

## What this does not do

**It does not fill the column.** The counts above are what they are: after this, most rows carry
no recap, because most sessions write none. Making a recap exist for every agent and every kind
is the other half of MUX-28, which stays open. It is a different piece of work, it costs tokens,
and it needs a setting to turn it off.

**It does not read the recap Claude Code shows in the pane.** It reads the entry that line is
written from, which is the same thing seen from the other side.
