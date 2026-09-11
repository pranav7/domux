# 0038: The activity sits one cell after the name, and the sidebar shows the glyph alone

**Date:** 2026-09-11
**Status:** Accepted. Amends decision 0030 (the dot's shape and place), decision 0032 (the
tick rate) and interface spec 6.2 (the gap).
**Decision:** Four things the author asked for on reading the Navigator. The waiting mark is
`◉`. The activity, glyph or dot, sits one cell after the name in every form. The two sidebar
forms draw a working or compacting row's glyph without its word. The animation tick is 70 ms
rather than 80.

## Context

The dot is the one thing that says an agent is waiting on you: M4's Notifier is out of scope
(the author's ruling of 2026-09-11), so nothing else draws attention to a blocked agent. MUX-29
shrank it to `•`, and at that size it did not stop the eye. `◉` is a ringed disc: bigger than
the small dot, and unlike the full circle MUX-29 replaced, it has a ring, so it reads as a mark
beside the name rather than a shape competing with it.

Interface spec 6.2 put two cells between the name and the activity. In a one-line row under a
workspace that gap read as a hole, which is why MUX-29 had already moved the dot in to one cell.
The glyph now sits at one cell too, so the dot and the glyph share a column and there is one
gap constant rather than one per kind of activity.

The sidebar is 38 columns, and a name beside a working word (`claude ✱ Whisking…`) crowded it.
In the sidebar the turning glyph says working, and its colour says compacting, so the two
sidebar forms draw the glyph alone. The overlays have the width and keep the word.
`RowForm::shows_word` is the one place that says which forms do.

The glyph turned every 160 ms, which was V1's rate (decision 0032). The author asked for
slightly quicker. One counter drives the glyph and the band (0032), so the tick moves from 80
to 70 ms and the glyph turns every 140 ms; a quicker glyph on an unchanged band would be a
second counter, which 0019 and 0032 both argue against. The harness floor is 50 ms
(`labels::ANIMATION_INTERVAL`), and 70 is clear of it.

## Consequences

- In the sidebar a working row and a compacting row differ by the glyph's colour only.
- The band moves a little quicker too, in the same ratio to the glyph as before.
- Every test that pinned the two-cell gap moved one cell, and the animation tests see nine
  turns over their fourteen looks rather than eight.
- The switcher's tail after the activity, the kind, the tab and the pane, keeps its two cells:
  `TAIL_GAP` is its own constant.
