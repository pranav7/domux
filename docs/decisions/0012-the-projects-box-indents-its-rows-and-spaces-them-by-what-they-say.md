# 0012: The Projects box indents its rows and spaces them by what they say

**Date:** 2026-09-09
**Status:** Accepted
**Decision:** A box pads its rows one cell in from each border. Under a project header every
workspace row is indented two cells, and an untouched slot's hollow glyph hangs in that indent
rather than standing in front of the handle. A blank row goes before each project header, and
inside a project between two workspaces when either of them drew more than one line.

## Context

MUX-9 reported three things about the box, with screenshots: the switcher's rows touch its
border, the list has no hierarchy because every row starts in the same column, and every row
is spaced the same way so nothing says what is nested inside what. The issue asks for the
spacing to differ between workspaces and projects, and for less of it.

Interface spec 5.2 said the opposite of two of these: "There is no gutter and no `›`. Rows are
flush with the box's left padding", and "One blank row between workspaces, and one before the
next header". The first sentence names a padding that did not exist, so rows sat against the
border. The second gives one gap to two different joins, which is what the report is about.

The comment on MUX-9 points at V1's switcher as the tone to take. Reading it: the project name
is at the left edge and the workspaces under it start two cells in; an untouched slot's glyph
sits in those two cells so that every name still starts in the same column; and a blank row
follows a workspace that had a branch, a pull request or an agent line under it, while
consecutive one-line slots are drawn tight.

That last rule is the one worth naming, because "no blank between workspaces" alone reads
badly wherever a row has a second line. It has to look at both sides. A first pass gave the
blank only to the row that was tall, and the author read the result as one workspace: `main`
on its own line, then a named workspace and its branch under it, with nothing between the
three. A one-line row directly above a two-line one is read as that row's first line, and the
workspace it really names disappears from the reader's count. So either side being more than
one line earns the gap, and a run of one-line slots still draws tight.

## Consequences

- `ListBox` owns the padding, so both surfaces and M3's Agents box get it from one place.
  `list_box::content_width` is what every caller measures a row against, so the width a row
  truncates to and the width it is drawn in stay one number.
- The fill band still spans the box's whole inner width. The pad is inside the band, not
  beside it, so a filled row still reads as one band to both borders.
- A row's text loses four cells to the pads and two more to the indent. In the sidebar that
  leaves 34 of 38, and in the switcher's 60-cell box, 54.
- `list_box::needs_gap_between` is the whole spacing rule. `projects_box::rows` writes the
  list to it and `filter_rows` rebuilds to it, so `/` changes what the list holds and never
  its shape.
- A row is its name and then its branch, and never the handle the name stands in for. That
  was already true, and it now has a test of its own, because the run-together above is what
  made it look untrue.
- Interface spec 5.2's two sentences no longer describe the build. This record is what
  replaces them.
