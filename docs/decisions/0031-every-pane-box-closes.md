# 0030: Every pane box closes

**Date:** 2026-09-11
**Status:** Accepted. **Reverses decision record 0022**, which is retired with it.
**Decision:** Every box draws its bottom rule, the pane box standing on the workpanel's last
row included. `Boxed::bottom_rule` and `render::pane_bottom_rule` are gone, and
`render::pane_screen` takes two rows of chrome off every rectangle it is given.

## Context

Decision 0022 answered MUX-14, which asked for less padding at the bottom of the workpanel.
There was no padding to take, so what it took instead was the bottom rule of the pane standing
on the screen's last row: that row carried no words, and the program could have it.

MUX-27 is the same screen a day later, with an arrow at the bottom left corner and the note
"bring the line back at the bottom of the workpanel, so the square looks complete". The rule
carries no words, which is what 0022 measured, but it closes the shape, which 0022 did not.
A box with three sides reads as a box that failed to finish drawing, and the side rules
running off the bottom edge of the screen are what make it read that way.

So the row is worth more as a line than as output, and the author who asked for the row back
is the author who asked for it in the first place.

## What it costs

One row of program output per pane, given back. A single pane on a 30 row screen gets 27 of
30: one row to the tab row, one to the top rule, one to the bottom rule. That is the number
0022 moved to 28 and this moves back.

## Why the whole special case goes rather than the flag flipping to true

`bottom_rule` had one caller that ever passed false. Keeping a field every box sets the same
way is a question nobody asks, answered in six places, and it is the sort of flag a later
change sets wrongly in one of them. Taking it out means:

- `Boxed` has no `bottom_rule` and `Boxed::inner_of` no second argument. Every box is closed
  and its inner area is two rows shorter than its rectangle.
- `render::pane_bottom_rule` is gone. Nothing asks the question any more.
- `render::pane_screen` takes a rectangle and no workpanel. It was given the workpanel only to
  answer whether the rectangle stood on its last row.

What 0022 got right stays: `pane_screen` is still the one place a layout rectangle has the
chrome taken off it, and `draw_panes`, `pane_hit`, `Core::sync_pane_sizes` and
`Core::provisional_size` all still ask it. Two answers to a pane's size is how the rows a
program is given stop being the rows drawn for it, whatever the chrome costs.

## Consequences

- The tests 0022 moved move back: the scrollback counts and screen rows in `tests/copy_mode.rs`,
  `tests/mouse.rs`, `tests/screen.rs`, `tests/sidebar.rs` and `tests/tabs_and_panes.rs` are the
  numbers they held before it. What they pin did not change; the geometry under them did.
- `render_primitives.rs` loses the open box and keeps one test that the rectangle a box reports
  is the rectangle it drew. That is the rule the open box was there to hold, and it holds for a
  closed box too: a box that drew a row it did not report would leave a blank line, and one that
  reported a row it did not draw would write over the rule.
