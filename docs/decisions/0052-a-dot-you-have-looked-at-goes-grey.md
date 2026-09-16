# 0052: A dot you have looked at goes grey

**Date:** 2026-09-16
**Status:** Accepted. Amends decision 0030 (the dot means waiting, and nothing else) and
decision 0026's split of `unseen` from the dot. MUX-52.
**Decision:** A waiting agent draws its dot for as long as it is stopped. The colour is
`unseen`'s: red until you have opened the agent's pane, grey after. `select_tab` clears
`unseen` the way `focus_pane` does, and every arrival at waiting sets it again.

## Context

The author, on a screen with one agent waiting: "once you open the pane in which the agent is
running the dot should clear, it means you've viewed the agent's message and it's now in ideal
state waiting on you." The row in the screenshot was the pane the author was sitting in, and it
carried the red dot anyway.

Decision 0030 made the dot the state's alone, because six rows carrying six dots in six colours
meant the reader had to read each one to find the one that wanted them. That fixed the crowding
and left a second problem: a dot says "come here" for as long as the agent is blocked, so a
prompt you have read goes on calling you until you answer it. With eight agents running, the
marks that mean "I have seen this, I will get to it" are indistinguishable from the one that
just arrived.

`unseen` already carried exactly the reading this needs. It turns on when an agent wants you and
clears when the agent's pane is focused or when input reaches that pane, which is what "you have
seen this" means everywhere else in domux. Decision 0030 left it with one job, brightening a
recap in the switcher, and said in as many words that it was worth a look of its own.

## Two questions, two marks

The dot answers "is this agent stopped?" and the colour answers "have you looked?". Splitting
them this way keeps both.

The alternative the author and the record weighed was taking the record from `waiting` to
`idle` when you open its pane, and giving idle a mark it does not have today. That was rejected
on three counts. The `reason`, the notification's own message, is cleared on leaving waiting, so
the record would forget what it asked. `agent.list` and `peek` would report idle for an agent
holding a permission prompt, which is a lie to anything scripting against them. And a mark on
idle is a mark on most rows, which is the crowding 0030 removed.

Dropping the dot entirely once seen was rejected too: a blocked agent you glanced at would then
look exactly like one that finished its turn, on every surface, and the author asked for an
affordance rather than for silence.

## The colour

`waiting_dot_seen` is a new role, so a theme sets it (decision 0042: a new colour is a new
role). It is guarded as a `Line` at `DOT_OFF_FLOOR`, the floor the stay awake dot for "not held"
uses, so the guards keep it quiet on a read ground rather than lifting it to text contrast: a
mark that says "no hurry" is not made to shout by a light terminal.

Its value is the faint text tier's in both built-in themes, `#6c7086` under `domux` and
`blend 4/9` under `terminal`. A step brighter than the stay awake off dot, which is the dimmest
thing domux draws, because this one still reports a blocked agent.

## What counts as opening the pane

What already cleared `unseen`: the agent's pane becoming focused, through `Model::focus_pane`,
which every focus route reaches, and input arriving at it, through the three `seen_by_input`
call sites. Reading the records never counted and still does not.

One route was missing. `Model::select_tab` moves a client to a tab and focuses that tab's
focused pane, and it cleared nothing. So `leader 2` and a click on the tab row put an agent's
pane in front of the reader with the focus in it and left the dot red. It clears now, on the
same reasoning `focus_pane` gives: every caller is a reader act.

## Every arrival at waiting is a fresh call

`attention` was `to == Waiting && from != Waiting`. Waiting to waiting was excluded because a
second notification from an agent that is already blocked changed nothing worth noticing while
`unseen` only brightened a recap.

It is worth noticing now. An agent that asks permission, waits while you read it and then sends
an idle prompt has called you twice, and you have seen one of them. So `attention` is
`to == Waiting || (from == Working && to == Idle)`, and the table test gains the waiting row.

## Consequences

- `Agent::needs_you` is `waiting && unseen`. Its one caller is `red_dot_count`, which feeds
  `red_dots` in `agent.list` and is documented as the red dots, so the count follows what is on
  the screen. The state still reads `waiting`, so a caller that wants every stopped agent counts
  the state instead.
- `sorted_agents` ranks an unseen waiting record above a seen one, above working. The agents
  overlay asks which agent wants you, so a fresh call sits above one you have read. The
  Navigator never sorts, so no row moves there.
- A theme file that does not set `waiting_dot_seen` takes it from the theme it extends, the way
  every other role works. No existing theme file breaks.
- The dot still marks a blocked agent on every surface, which is what makes this safe: walking
  away from a prompt you have read does not make it disappear.
