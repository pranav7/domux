# 0019: The switcher pads more than the sidebar, and neither lists tabs

**Date:** 2026-09-10
**Status:** Accepted. Amends decision record 0012, whose other rules stand.
**Decision:** A workspace row is its name and its branch line, on both surfaces. The tab list
under it is gone. `ListBox` takes its padding as a value rather than a constant: the switcher
pads two cells in from each border and leaves a blank row under its top rule and above its
bottom one, and the sidebar keeps the one cell decision record 0012 gave it and no blank rows.

## Context

MUX-9 was about the Projects box in the sidebar, and decision record 0012 answered it: one
cell of padding, a two-cell indent under each project header, and a blank row between two
workspaces only where one of them drew more than one line.

MUX-12 is the same reading of the switcher, with three asks: drop the tab list, tighten the
spacing between workspaces, and give the text room to breathe at the top and the sides.

The three are one change. Every workspace in the switcher drew a tab list, so every workspace
was at least two lines, so `needs_gap_between` gave a blank row on both sides of every one of
them. Taking the tab list out tightens the spacing on its own, and what is left is a list
where a one-line workspace sits tight against its neighbour and a workspace with a branch is
parted from both, which is what 0012 wanted in the first place.

## Why the tab list goes rather than moves

Interface spec 5.5 gave the switcher the tab list because it had the width for it. Width is
not the test. The line says how many tabs a workspace has and what any named ones are called,
and the reader is looking at this box to choose a workspace, not a tab: the tab row shows the
tabs of the workspace they land in, a moment later and in the place they will act on them. So
the line cost a row of every workspace and answered a question nobody was asking there.

## Why the two surfaces pad differently

The switcher's box is 60 cells and the sidebar's is 38, and the sidebar spends what it has on
branch names: at one cell of padding a sidebar row has 34 cells of text and 32 after the
indent, and `feat/tod-per-line-extraction` is 28 of them. Two cells each side would take two
more from every row on the surface where they are scarcest, to fix a complaint about the
surface where they are not.

So the padding is a value the caller passes. `SIDEBAR_PAD` and `OVERLAY_PAD` are the two, both
named in `list_box`, so a third surface picks one rather than inventing a number, and M3's
Agents box will take `SIDEBAR_PAD` by sitting in the same column.

## Consequences

- `content_width`, `text_area` and `box_lines` all take the pad, and the drawing, the pointer
  hit test and the `list.*` handlers that walk the cursor all measure through them. A box the
  reader sees and a height the cursor is walked against that disagreed would put the cursor on
  a row nobody can see.
- `box_lines` never answers less than one row of text, because a box with no rows draws its
  empty text on the first row it has. Without that floor an empty switcher sized itself to its
  padding and said nothing at all.
- `Extras::wide` now means only the pull request title. The sidebar and the switcher differ in
  that one line and in their padding, and in nothing else.
- Interface spec 5.5's tab list sentence no longer describes the build. This record replaces
  it, as 0012 replaced 5.2's.
