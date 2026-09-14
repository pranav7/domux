# 0048: A compacting row draws an arrow that breathes

**Date:** 2026-09-14
**Status:** Accepted. Amends decision record 0038, under which a compacting row in the sidebar
differed from a working row by the glyph's colour alone.
**Decision:** A compacting row draws `↓` where a working row turns the star. The arrow holds
still and its colour breathes, from `band_compacting_dim` through `compacting` to
`band_compacting_bright` and back, once every 28 ticks of the one animation counter. The word
`Compacting…` keeps its band where a form draws the word.

## Context

MUX-48 asks for an arrow that animates up and down while an agent compacts, in place of the
star. Decision 0038 took the word off the two sidebar forms, which left the star's colour as the
only thing that told a compacting row from a working one. A lavender star in the Navigator did
not read as a different state from the orange one on the row above it.

A terminal draws a glyph in whole cells, so an arrow cannot rise and fall by part of a line. The
author was offered three ways to fake the motion on 2026-09-14: flip the arrow between `↓` and
`↑`, alternate a solid `↓` with a dashed `⇣`, or keep `↓` still and breathe its colour. The author
chose the breath.

## The breath

`render::shimmer::breath` answers how lit the arrow is at a tick, from 0 at the dim end to 1 at
the bright end. It is a cosine rather than a straight rise and fall, so the colour lingers at
each end and hurries through the middle. That reads as breathing, where a straight line reads as
a colour sliding back and forth.

A breath is 28 ticks, 1.96 s at 70 ms. Four to a quarter, so a quarter of a breath lands on a
whole tick, and at that tick the arrow is exactly the `compacting` colour.

The breath reads the counter the star and the band already read (decision records 0019 and 0032).
A second ticker would be a second piece of state that can disagree with the first, which is the
argument both records make.

## Three colours, not two

The breath could have run between the band's two ends alone, the ends the word's band runs
between. That would have left the `compacting` role drawn nowhere. Role names are public
(decision record 0042), so a theme file that sets `compacting` would set nothing, and the role
could not be removed without a record of its own.

So `theme::Breath` holds three colours and passes through the role half way. Under `domux` the
role is almost exactly half way between the two ends anyway, `#afafff` against `#a4a4e7`, so the
breath looks as a two colour one would. A theme that sets the role to something else sees the
arrow pass through it.

## The glyph

`↓` is U+2193, and both fonts on the author's Linux machine carry it: CaskaydiaCove Nerd Font,
which Ghostty uses there, and JetBrains Mono Nerd Font. `⬇`, `⤓` and `ꜜ` are not in
CaskaydiaCove. A glyph from a fallback font comes out at another size and weight beside the name.

## Consequences

- In the sidebar a compacting row and a working row differ by shape as well as colour.
- `labels::COMPACTING_GLYPH` names the arrow. `render::agents_box::working` takes its glyph
  already styled, so working and compacting share the word and its band and differ only in the
  glyph.
- The animation ticker runs while an agent compacts, as it did before: the breath needs it.
- The theme probe has a `Breath` check. The arrow is a different colour on each look, so the
  probe accepts any colour on the way from the band's dim end through the role to its bright
  end, and nothing else.
