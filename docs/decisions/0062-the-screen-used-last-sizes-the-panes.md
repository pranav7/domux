# 0062: The screen used last sizes the panes

**Date:** 2026-09-29
**Status:** Accepted. Replaces the rule that the smallest screen on a tab sizes its panes.
**Decision:** When two or more clients show one tab, its panes take the size of the one that was
used last. A larger screen leaves the rest blank, as before. A smaller one draws the boxes at the
agreed size and cuts them off at its own edge, until it is used and the panes take its size.

## Context

MUX-53: a screen in iTerm drew its pane boxes some twenty columns short of its right edge and left
the rest blank, while the top bar reached the edge. The rule then was tmux's old one: every client
on a tab agrees on the smallest workpanel among them, so one narrower screen attached anywhere,
such as a Ghostty tab left open beside the iTerm one, shortens every other screen on that tab. The
screen being read is the one that loses.

tmux itself moved off that rule: `window-size latest` has been its default since 3.1. This is that
rule.

## What counts as use

Whatever `Core::client_input` already counts through `Model::touch_client`: a key, a paste, the
mouse and the wheel, a resize and attaching. It is the same activity that decides which client an
API call without a `client` answers for, so the two cannot disagree about which screen is in front.

The terminal's focus counts only when it arrives. A focus-out is the reader leaving the screen, and
counting it would let a focus-out that lands after the other screen's focus-in hand the panes back
to the screen just left.

## The smaller screen

The rectangle the clients agree on can be larger than a client's own workpanel, so `draw_panes`
no longer trusts it to fit. `Boxed::render` and `render_grid` index the buffer without checking,
and an area past its edge panics rather than clips; a workpanel that does not fit is drawn on a
buffer large enough to hold it, and only this screen's cells are copied back. The cursor is kept
only when it lands on this screen.

The boxes are cut off rather than squeezed, because a squeezed box would draw a program's screen
at a size it was not given. The missing right border says there is more past the edge.

## Consequences

- One function answers the agreed size, `render::latest_among`, and every caller already went
  through it: the renderer, `Core::sync_pane_sizes`, `Core::provisional_size` and the API's
  `tab_area`, so a pane's program and every screen still agree on its size.
- Moving between two screens of different sizes resizes the panes each time, and the programs in
  them redraw. That is tmux's behaviour under the same rule.
- `Core::sync_pane_sizes` marks every view dirty when it resizes a pane, because every screen on
  the tab draws the boxes at the new size, not only the one whose input moved it.
