# 0067: The Navigator starts level with the panes, under one bar

**Date:** 2026-11-26
**Status:** Accepted. Amends decision record 0064: the sidebar and the first pane still share
one line, and that line now runs from the row under the bar, with the two top lines joined.
**Decision:** While the sidebar is shown, the sidebar's column starts on row 1, the row the pane
boxes start on. Row 0 is one bar across the whole screen: `project › workspace` over the
sidebar, then the tabs over the panes, then the right end.

## Context

The author pointed at the corner where the Navigator met the tab row. The Navigator's box took
the whole height of the screen, so its top line was on row 0, beside the tab row, while the pane
boxes started on row 1 under the tab row. There were two top lines one row apart. The Navigator's
`┐` stood up beside the tabs, the current tab's accent fill touched it, and the pane's `├` sat
one row below.

Two designs were drawn as artboards:

- **A.** Keep the boxes. Move the Navigator down one row so its top line and the panes' are one
  line, and give row 0 to one bar across the screen.
- **B.** Drop the boxes for flat regions divided by single lines, with one header row, after a
  desktop app the author pointed to.

The author chose A. B lost the accent border round the focused pane, which is what makes domux
look like domux.

## The bar

`tab_row::draw_workpanel_row` fills all of row 0 on `tab_row_background`, from column 0 to the
right edge. Over the sidebar it writes `project › workspace`, the label the full-width top bar
opens with, in bold `text`, and cuts it two cells before the tabs. The tabs start where they
started before, one column after the line the sidebar and the panes share.

The ground is `tab_row_background` and not `top_bar_background`. The role names the row that
carries the tabs while the sidebar is shown, and a theme that sets it keeps working. Under both
built-in themes it is the terminal's own ground.

The label is not a target. A click on row 0 asks the tab row and nothing else, so a click over
the sidebar's part of the bar acts on nothing, as a click on the full-width bar's label does.

## The column

`sidebar::sidebar_area` is the rectangle every other measure of the sidebar comes from: the
split into boxes, the hint row, the pointer and the list handlers. It starts on row 1 and is one
row shorter, so all of them move with it. The Navigator shows one row less of its list.

The Navigator's top line and the first pane's meet in `┬`, by the join 0064 added, and their
bottom lines meet in `┴` as before.

## Consequences

- Every frame with the sidebar shown changes: row 0 carries the label, and the sidebar's boxes
  are one row lower and one row shorter.
- Showing or hiding the sidebar no longer moves the tabs between rows. They stay on row 0 and
  move along it.
