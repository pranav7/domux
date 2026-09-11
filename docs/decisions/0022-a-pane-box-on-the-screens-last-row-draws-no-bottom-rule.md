# 0022: A pane box on the screen's last row draws no bottom rule

**Date:** 2026-09-10
**Status:** Retired by decision record 0030, which closes every pane box again and takes
the open box out of `Boxed` altogether. Everything below is what the build did until then.
**Decision:** A pane box whose last row is the workpanel's last row draws no bottom rule. Its
side rules run to that row and the row holds the program's output. A box with another box
under it keeps its rule. `render::pane_bottom_rule` is the one answer, and
`render::pane_screen` is the one place a layout rectangle has the chrome taken off it.

## Context

MUX-14 asked for less padding at the bottom of the workpanel, with a screenshot of a full
height Claude Code pane and the note "get this vertical back a bit so more content can
render".

Measured, the workpanel already runs to the last screen row and there is no padding anywhere
in it. On a screen of 30 rows the program gets 27: one row goes to the tab row, one to the
box's top rule, and one to the box's bottom rule. So the ask is about which of those three is
worth its row, and only one of them is not.

The top rule carries the pane's command and its flags, which is the sentence that says what
is running and whether it is zoomed, in copy mode or exited. The tab row carries the tabs. The
bottom rule carries nothing. It is worth a row where it parts one pane from the pane under it,
and where it stands on the last row of the screen it parts the pane from the edge of the
terminal, which needs no line drawn on it.

## Why the workpanel's bottom and not the screen's

The rule is written against the rectangle the layout was solved into, not against the client's
own screen height. Two clients on one tab agree a workpanel the size of the smaller of them,
so on the larger screen the boxes stop short of its last row. Asking about the screen there
would drop the rule from a box with blank rows under it, and worse, the renderer and
`Core::sync_pane_sizes` would answer differently: the program would be given one row more or
fewer than the box has room for, which shows as a blank line or as output written over the
rule. Asking about the workpanel keeps every caller on one answer.

## Consequences

- `Boxed` gains `bottom_rule`. Every other box in the build passes true: the sidebar's, the
  switcher's, the name box, the confirmation, the help overlay. Only `draw_panes` computes it.
- `Boxed::inner_of` takes the flag too, because the inner area is what the grid is copied
  into, what the pointer hit test measures against, and what the PTY is sized to. A box that
  drew the extra row without reporting it would leave a blank line; one that reported it
  without drawing it would write over the rule.
- `Core::provisional_size` and `Core::sync_pane_sizes` both go through `render::pane_screen`.
  They each subtracted their own constant before, and only one of them was found when this
  change was first made. A pane spawned into a tab no client draws was then a row shorter than
  the same pane once a client looked at it.
- Every pane is one row taller than it was, so the tests that count rows of scrollback, name
  the line at a screen row, or assert a pane's size moved by one. The behaviour they pin did
  not change; the geometry under them did.
- A single pane on a 30 row screen now gets 28 of 30 rows.
