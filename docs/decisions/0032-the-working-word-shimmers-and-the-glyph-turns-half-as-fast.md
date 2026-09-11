# 0031: The working word shimmers and the glyph turns half as fast

**Date:** 2026-09-11
**Status:** Accepted. Amends decision record 0019, which named the animation timer after the
glyph alone.
**Decision:** A bright band runs along a working agent's word, V1's `shimmerText` carried over
arithmetic and colours both. It moves on every 80 ms tick, and the glyph holds each of its
frames for two of them. The corner an agent row wears under its workspace is `⌞` rather than
`↳`.

## Context

MUX-26 asks for V1's shimmer back and for less movement on the glyph. They are one change:
V1's glyph already advanced every second tick (`renderAIBadges`, "icon advances every 2
ticks"), and it did so because the band needs the fast tick to glide and a glyph turning at
that rate flickers under it. Running the glyph at the tick rate was what made it restless, so
the fix for the glyph is the rate V1 shipped, not a slower ticker.

## One counter, two animations

`AgentsState::tick` counts up once per `labels::ANIMATION_INTERVAL` while anything works, as it
did under its old name `glyph_tick`. `AgentsView` now carries that count rather than the
resolved glyph, and the row reads both animations off it: `labels::frame_at` divides by
`GLYPH_TICKS_PER_FRAME`, and `render::shimmer::lit` takes the count as it is.

A second ticker for the band would be a second piece of state that can disagree with the
first, which is the argument decision 0019 already made against a ticker that starts and stops
with the agents. It would also let the two animations drift apart on a loaded server, and they
are drawn two cells from each other.

Handing the renderer the count rather than the glyph is a departure from "everything the row
needs already looked up". It is what the band forces: there is no resolving a band to hand
over, because it is a colour per character and the renderer is what walks the characters.

## The colours are V1's, picked by eye

V1 declares a dim and a bright end per kind and for compacting, and they are not derived from
the kind's colour: `#B85E47` to `#FFC9B0` for Claude where the kind is `#DE7356`. Scaling the
kind's colour instead lands within a few units of three of the four pairs and well off the
fourth, so a rule would be a rule that had to be corrected by hand anyway.

`AgentKind` is three variants and `theme::agent_color` matches all three, so there is no open
set of kinds waiting for a pair to be derived for it. A fourth kind will declare a colour in
its manifest and a pair here, and both are one line.

The band itself is arithmetic and lives in `render::shimmer`; the two ends are colours and
live in `render::theme` beside the kind colours, as `Shimmer`. `Shimmer::at` mixes them, which
is the one place a colour between the two ends comes from.

## What is not carried over

V1 floors the length of a pass at twelve characters. Two tails already come to twelve, so a
word of one character makes a pass of thirteen and the floor can never bind. It is dropped
rather than copied as a line that cannot run.

## Consequences

- The glyph turns every 160 ms. The two rate-dependent tests in `tests/agents_animation.rs`
  sample fourteen times rather than eight, so the number of turns they see is the number they
  saw before.
- A working row is one span per character where it was one span for the word. Every caller
  measures a row by summing `display_width` over its spans, so the row measures what it
  measured; a caller that counted spans would not, and there is none.
- The glyph stays the kind's colour and is outside the band. It stands in a column with the
  glyph of every other row, and a band would take it out of that column for part of each pass.
- The band is off the word at each end of a pass, so the word is briefly all one dim colour.
  That is V1's `tail`, and it is why the frame test samples over a pass rather than at one
  moment.
