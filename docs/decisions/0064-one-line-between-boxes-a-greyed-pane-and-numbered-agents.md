# 0064: One line between boxes, a greyed pane, and numbered agents

**Date:** 2026-11-26
**Status:** Accepted. MUX-60. Replaces interface spec assumption 18 and the one column of gap
interface spec 12.18 put between the sidebar and the workpanel. Moves the tabs off `leader 1` to
`leader 9`. **Amended by decision record 0065**, which adds `D-1` to `D-9` for the agents and
writes their numbers only while cmd is held, at a client that can tell. **Amended by decision
record 0067**, which starts the sidebar on row 1, level with the panes, under one bar.
**Decision:** Neighbouring pane boxes share their facing line, and so do the sidebar and the
first pane, with the lines joined where they meet. Every pane but the tab's focused one is mixed
30% towards the terminal's background. `leader 1` to `leader 9` open the agent the Navigator
numbers so, the Navigator writes that number faintly at the end of the agent's row, and the tabs
move to `M-1` to `M-9`.

## Context

MUX-60 asked for three things, against screenshots of domux beside a desktop terminal app:

1. "Reduce the gap between the navigator and the terminal workbench", and lay them out with
   "basically no gaps". Between the sidebar and the first pane there were three columns: the
   sidebar's border, a blank column, and the pane's border. Between two panes there were two
   borders side by side, since `GAP` went to 0.
2. "The pane in focus is 100 opacity and the other is slightly greyed out."
3. "`<leader> 1 2 3` should allow to jump between these claude sessions, with a very very subtle
   indicator on what the number is for the claude session."

## One line between boxes

The solver (`layout::split_areas`) gives two children one more cell than their parent and puts
the second on the first's last cell, so a split's two boxes share a line (`layout::SHARED`). The
workpanel starts on the sidebar's last column for the same reason. Each pane gains a column for
each line it now shares.

`Boxed::border` with `join` meets a line already drawn rather than covering it. Each border cell
is a set of arms, up, down, left and right; a cell drawn onto one that already holds a box glyph
takes the union, so two corners on a shared edge become `┬` or `├`, and four become `┼`. Only
the pane boxes join. An overlay or a toast drawn over a pane's text does not, because a `│` in
the text is not a line to meet.

The colour of a shared line is the accent when either box beside it has the keys. A box that is
not focused keeps the colour it finds on a line it shares, and the focused pane's border is
drawn last. The sidebar is drawn before the panes, so its accent survives the panes beside it
while the Navigator has the keys.

`render::draw_boxes` now draws in three passes: every border, then every title and flag, then
the grids. A title sits on its box's top border, which with stacked panes is the bottom border
of the box above, and a border drawn after it would cover it.

The tab row starts one column into the workpanel, after the shared line, so a tab never sits on
the sidebar's corner. `render::workpanel_row_span` answers for the drawing and the pointer.

Focus moves measure across the shared line: a pane faces another when it starts on that pane's
last cell, and two panes that meet only at a corner of the line do not face each other.

## A greyed pane

A terminal has no opacity. What a pane at 70% over the terminal's background looks like is each
of its colours mixed 30% of the way to that background, so `render::dim` does that to every cell
of every pane but the tab's focused one. The colours come from the terminal's answers at attach:
OSC 10 and 11 for the defaults, OSC 4 for the sixteen palette slots, and the fixed 6x6x6 cube and
greys for slots 16 to 255. A cell on the terminal's own background is left on it, so it follows
the terminal.

When the answers did not arrive, the cell is drawn faint (SGR 2) instead. Ghostty draws faint
text at reduced opacity, which is close to the same thing; other terminals vary.

The pane dimmed is every pane but `tab.focused`, not every pane without the keys. While the
Navigator has the keys the tab's focused pane stays at full strength, so opening the sidebar does
not grey the whole workpanel.

`[terminal] dim_unfocused` sets the percentage, 30 by default, and 0 turns it off. It is not a
role: it adds no colour of its own, and a tab with one pane draws no dimmed cell.

## Numbered agents

`Model::navigator_agents` lists the agents in the Navigator's order: projects by name without
case, workspaces in their own order, and each workspace's agents in the order they started. An
agent's number is its place in that list, from 1. The number is read off the Navigator, so the
order is the Navigator's: a key that opened the third agent of another list would open a row the
reader cannot see the number of.

The Navigator listed a workspace's agents in the agents overlay's order, which moves as states
change, while decision record 0030 says they sit in the order they started. They now do, which is
also the order of their numbers.

`agent.focus #3` names agent 3. The `#` keeps a number from reading as a workspace someone named
`3`, since `agent.focus` already takes a workspace. The keys are `leader 1` to `leader 9`, bound
to `agent.focus #1` to `#9`, so a key, `domux agent focus '#3'` and the API reach the same
handler.

The number is the last cell of the agent's row in the Navigator, after at least one blank, in
`faint_text`, the colour of the `└` the row starts with. Only 1 to 9 are drawn, because no key
reaches 10. The agents overlay draws none: it sorts by who wants you, and a number there would
not be the one the key opens.

## The tabs move to `M-1` to `M-9`

`leader 1` to `leader 9` were `tab.select 1` to `9`. MUX-60 asked for the digits, and the author
chose `M-1` to `M-9` for the tabs, a global key with no leader, the way `C-h` is. A tab is still
one chord away, and a click on the tab row still selects it. The Keys overlay collapses both runs
into one row each, `leader 1-9 agent.focus #<n>` and `M-1-9 tab.select <n>`.

## Consequences

- Every frame with the sidebar shown, or with two panes, changes: the panes are wider and the
  lines between them single.
- A PTY beside the sidebar is two columns wider, and one beside another pane a column wider.
- A reader who pressed `leader 2` for the second tab now opens the second agent. A config that
  binds the digits keeps its own bindings.
- `M-1` to `M-9` no longer reach a program in a pane. Readline's digit arguments are the one
  common use, and a config that needs them unbinds the keys with an empty action.
