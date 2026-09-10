# 0028: One Navigator, and live sessions only

**Date:** 2026-09-10
**Status:** Accepted. Replaces decision 0018, replaces the exited half of decision 0026, and
amends interface spec 5, 6 and 10.
**Decision:** The Projects box and the Agents box become one box, `Navigator`, in the sidebar
and in the switcher. An agent is a row under the workspace it runs in. A record ends when its
session ends, so resume, dismiss and every exited row are removed. A dot is drawn only while an
agent is waiting. `[navigator] enabled` is on by default and exists for one release.

## Context

MUX-25. The report is one sentence: "I don't care about agents on their own. The context of
where the agent, which project, which workspace is more relevant to me for navigation."

M3 built two boxes because the agent list and the project list answer different questions.
Decision 0026 sharpened that split, giving the sidebar the running sessions and the overlay
every record. What MUX-25 says is that the split was the mistake. An agent is not a thing you
look up; it is a thing that is happening somewhere, and the somewhere is how you find it.

Artboard 9 is the drawing, and it is the specification for every row grammar below.

## The Navigator is one box on both surfaces

Projects alphabetically, `main` first inside each and then the slots, and under each workspace
the agents running in it. Nothing else changed about a workspace row: the name, then the branch
and the pull request, and the hollow glyph on an untouched slot.

An agent row is four cells in: the two of workspace indent every workspace row already has,
plus two for the arrow. Then the name, or the kind when nobody has named the session, then
the activity. Its project and its workspace are the rows above it and are never repeated, which
is the whole point of nesting it. The sidebar stops there. The switcher adds the kind and the
tab after the activity, and the recap on a second line, because it has the width.

**One cursor walks both kinds of row.** Enter on a workspace row switches to the workspace, as
it did. Enter on an agent row focuses the pane that agent runs in. `api::agent::focus` and
`api::workspace::focus` are unchanged; the list only says which row the keys are on.

**Nothing reorders.** Projects stay alphabetical and workspaces keep their order, so a row does
not move under the reader while an agent changes state. Inside a workspace the agents sit in
the order they started. `Model::sorted_agents` keeps its attention order for `agent.list` and
`peek`, which are read as lists of agents rather than as a place to navigate.

## A dot means waiting, and nothing else draws one

Decision 0026 made the dot red for waiting alone but left every other state drawing a dot in
its own colour. Six rows then carried six dots and the reader had to read each one to find the
one that wanted them.

So the dot is drawn only while an agent is waiting on you. Working and compacting already say
themselves with the animated glyph and the word, in the agent's colour and in periwinkle. Idle
and unknown say nothing, because nothing is happening.

**It sits where the working word sits**, two cells after the name. A waiting agent draws no
word, so the slot is free, and a mark in front of the name would push that name out of the
column every other row keeps it in. Interface spec 6.1's "never empty of dots" is retired: a box
with no dots in it is now the ordinary case and it means nobody is blocked.

## A record ends when its session ends

This is the larger half of the change and it is what principle 0 asks for.

An exited record existed so that resume had something to name. Resume typed
`claude --resume <session_id>` into the pane the session had run in, and it needed five
things to be true at once: the record exited, the kind Claude, a session id from a hook, the
original pane still present, and that pane free. Codex and OpenCode could never satisfy the
second, and a record the observer made could never satisfy the third. What the reader met was
mostly the refusal.

So resume goes, and with nothing left to name a dead session, the record goes when the session
does. What that deletes:

- `agent.resume` and `workspace.resume`, their CLI subcommands, and `agents::resume`.
- `[resume]` and the pass that resumed every workspace when the server started.
- `resume_command` in the manifests, and `RESUME_UNAVAILABLE`.
- `agent.dismiss` and its CLI subcommand. Dismiss removed an exited record, and there is none.
  `Model::remove_agent` keeps the removal itself, because the id retirement and the working
  word it hands back are still needed on the path that ends a record.
- `AgentState::Exited`, and with it `Liveness` and `AgentState::is_live`. Every record is live.
- `observer::prune_unresumable`, whose whole subject was a record nothing could resume.
- Agent records in the state file. Every process dies with the server, so every record restored
  was dead on arrival, and `mark_agents_exited_on_restore` existed only to say so. State schema
  5 removes the `agents` key; decision 0029 had taken 4 for the stay awake flag.

`transition` answers `Option<AgentState>` now. `None` is the record ending, which is what
`SessionEnd` and `ProcessGone` mean from every state. The table keeps its property: a state is
a row, an event is a column in every row, and the table is five rows now rather than six.

`attention` loses one of its three triggers, because a record that exits is not there to be
noticed. It turns `unseen` on when an agent starts waiting and when it goes from working to
idle.

**`unseen` is left in place and is worth a look of its own.** It had three jobs: lift a row in
the sort, brighten its recap, and colour its dot. Decision 0026 took the dot, and dropping
attention ordering takes the sort, so brightening a recap in the switcher is all that remains.
That is a removal to weigh on its own evidence rather than to fold into this one.

## What a lost hook looks like now

A `Stop` that never arrives still leaves a row saying `working` with a turning glyph, and that
is still what hook loss looks like. What has changed is the end: the row goes when the process
does, rather than settling into an exited row that stays until it is dismissed. Going and
looking is still the answer, and the row disappearing is a second way to notice.

## `[navigator] enabled` is on by default and is temporary

```toml
[navigator]
# One list of projects, workspaces and agents, in the sidebar and in the switcher.
# false restores the separate Projects and Agents boxes and the agents overlay.
enabled = true
```

Off, the sidebar draws the two boxes and `leader a` opens the agents overlay. On, the sidebar
draws one box and `leader a` does nothing at all: silently, because a key that only ever refuses
is a key with nothing to say, and the reader who presses it is about to find the same list in
front of them.

Two layouts is what principle 0 argues against, and the key exists anyway so that the author can
live with the new one before the old one is deleted. It is written down here as temporary so
that the deletion is a decision already taken rather than one to be argued again.

Off is the smaller claim than it looks. The two boxes keep their rows, their keys and their
region kinds; what they do not get is any new behaviour. The dot rule, the record lifetime and
the loss of resume are the whole build's, not the Navigator's, and they apply either way.
