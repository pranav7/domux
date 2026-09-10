# 0025: The Projects box takes the rows it needs, and `C-h` reads the pane's first row

**Date:** 2026-09-10
**Status:** Accepted. Amends M3 plan assumptions 21 and 25.
**Decision:** The sidebar's Projects box is as tall as its project rows need, up to half the
column, and the Agents box takes everything left over. `C-h` from a pane enters the Agents box
when that pane starts at or below the Agents box's first row, and the Projects box otherwise.

## Context

MUX-18: "too much padding between projects and agents". The reader had four projects, and the
Projects box drew them in five rows and then eight blank ones, because plan assumption 21 gave
each box half the column whatever was in it. The gap the arrow pointed at was the dead space
inside the upper box, not the one row between them, which is one row and stays one row
(interface spec 12.18).

The Agents box below it was scrolling at the same time. So the change is one change: the upper
box stops holding rows it has nothing to put in, and the lower box, which is the list that grows
over a day's work, gets them.

## The rule

`render::sidebar::split_column` takes the height the Projects box asks for and clamps it three
ways, in this order:

1. **Never more than half the column.** Past half the Projects box scrolls like any other list.
   Without the cap, a reader with twenty projects would squeeze the Agents box to its five-row
   minimum, which has room for one agent. Half was the old answer at every height, and it is
   still the answer for anyone whose project list is long.
2. **Never less than `MIN_PROJECTS`**, so an empty list still has room to draw its own empty
   text.
3. **Never so much that the Agents box drops below `MIN_AGENTS`.** This is plan assumption 21's
   own edge and it is unchanged: on a column too short for both minimums the Agents box keeps
   its five rows, because an Agents box below five has no room for a row at all where a short
   Projects box still shows a project.

`wanted_projects_height` reads the whole project list and never a filtered one, so the boundary
between the two boxes does not move while the reader types in either box. `/` shortens what is
in a box, not the box.

`split_for` is the one function every caller goes through: the drawing, the pointer's hit test
and the two handlers that walk these rows. Nobody measures the split a second way.

## Why `C-h` changed with it

`region_for_rows` used to ask which box a pane's rows overlapped more, with the row between the
boxes counted as Projects' territory and ties going to Projects. That answered "Projects" for a
pane filling the workpanel, which is the commonest layout and what interface spec 12.29's own
frame 8.2 shows, but only because the two boxes were equal halves: the answer rested on the row
between them leaning to Projects by one and on a round-up in the split leaning the same way at
odd screen heights. Both arguments die with the equal halves. A Projects box of eight rows over
an Agents box of twenty-five would have sent every full-height pane into the lower box.

The rule now is the pane's own first row. A pane that starts beside the Projects box enters
Projects; a pane that starts at or below the Agents box's first row - the lower pane of a
vertical split, which is the case interface spec 12.29 is titled for - enters Agents. It is a
fact about the pane rather than about how tall the boxes happen to be drawn, so adding a project
cannot change where `C-h` goes.

Two things move with it, both deliberate:

- **At 10 and 11 rows** the minimums bind, the Agents box is the larger of the two, and the old
  rule sent a pane filling the workpanel into it. The new rule sends it to Projects, like every
  other height. That edge was recorded as a decision rather than a gap, and this is the same
  decision made the other way: one answer at every height reads better than an answer that turns
  over on two of them.
- **A pane that starts beside Projects and reaches well into the Agents box** now enters
  Projects. Under the overlap rule it entered Agents. `C-h` is reached for to get at the
  sidebar, and Projects is the surface it is reached for.

`a_pane_filling_the_workpanel_enters_projects_at_every_height` sweeps every screen height the
program can be on against every height the Projects box can ask for, and reports the pairs that
disagree rather than stopping at the first. The defect this replaces was not a test that could
not fail; it was a suite whose fixtures were all even-numbered heights, where the old rule
happened to answer correctly.

## What was considered and not done

**Dropping the row between the boxes instead.** It is one row, and interface spec 12.18 asks for
it. It also is not what the reader was pointing at.

**Sizing both boxes to their content.** The Agents box's contents change all day, so the
boundary between the two would move under the reader every time a session started or ended. A
boundary that moves is worse than one row of dead space.
