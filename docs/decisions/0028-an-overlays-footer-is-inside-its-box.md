# 0028: An overlay's footer is inside its box

**Date:** 2026-09-10
**Status:** Accepted. Amends interface spec 3.2 and the glossary's "footer", whose other rules
stand.
**Decision:** The switcher and the agents overlay draw their footer on the box's last inner
row, inside the border, in by the same pad the rows above it start from. The box asks for that
row in `box_lines`, gives it up in `text_area`, and hands it back through `footer_area`. The
sidebar's hint row is unchanged: it sits under both of the sidebar's boxes and belongs to
neither.

## Context

MUX-16 is one screenshot of the switcher with a filter open. The box ends in its bottom rule
and the footer stands on the row below it, over the dimmed panes, with the pane's text one row
further down. The reading is that the border says nothing about where the overlay ends: the
keys under it could belong to the box or to the screen, and the eye has to work out which.

Interface spec 3.2 put the footer there. It reads "the footer under the box", against the
sidebar's "the hint row under the boxes", and the two were written as the same shape on two
surfaces. They are not the same shape. The sidebar's hint row is under two boxes and serves
both, so it can belong to neither. An overlay has one box, and its footer serves that box
alone, so a row outside the border is a row that looks like it belongs to nothing.

## Why inside rather than a second border

The other three overlays already draw their keys inside their border. The name box (interface
spec 7.1) ends in `⏎ save    esc cancel    an empty name clears it` on its last inner row, the
confirmation does the same, and the keys overlay closes with `esc close`. So the shape this
record chooses is the one three of the five surfaces were already drawing, and the change ends
a split rather than starting one.

A rule between the rows and the footer was the other option. It was dropped: it costs a row on
a box whose height is already clamped to the screen less six, `Boxed` draws no tee join today,
and the pad row that already sits above the bottom rule parts the two well enough. The name
box makes the same choice with the same gap.

## Why the pad and not one cell

The footer used to keep one cell of padding of its own, the same as a box's title. The rows
above it stand at `Pad::side`, which is two cells on an overlay, so a footer with one cell
stood a column to the left of everything over it. `footer_area` takes the side pad off both
ends and the footer draws from the first column it is handed, so the two agree by construction
and there is no second number to keep in step.

## Consequences

- `Pad` carries a third field. `OVERLAY_PAD` keeps the last row, `SIDEBAR_PAD` does not, and
  every function that already took a pad now answers for the footer row too: `box_lines` asks
  for it, `text_area` stops one row short of it, and `footer_area` returns it. The page the
  cursor moves by is `text_area`, so `list.up` and `list.down` walk the rows the reader can
  see and not the rows plus the footer, without a rule of their own.
- The overlay is one rectangle. `list_overlay_area` no longer keeps a screen row under the
  box, `dim` is told to keep one rectangle rather than two, and `overlay::footer` no longer
  clears its own row, because the overlay cleared the whole box before the box drew.
- A box is one row taller for the same rows. On a screen too short to grow, the rows give up
  the row instead: an overlay on a 12 row screen shows one line of rows where it showed two.
- Interface spec 3.2's "the footer under the box" and the glossary's "the row under an
  overlay's box" no longer describe the build. This record replaces both.
