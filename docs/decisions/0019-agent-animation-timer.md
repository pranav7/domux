# 0019: The agent animation timer always runs

**Date:** 2026-09-10
**Status:** Accepted. **Amended by decision record 0031**, which puts a second animation on
the same ticker: `GLYPH_INTERVAL` is `ANIMATION_INTERVAL`, `agents.glyph_tick` is
`agents.tick`, and the glyph turns on every second tick rather than every one.
**Decision:** The 80 ms ticker starts with the server and runs until the server stops. It
never starts or stops with the agents. The core turns the glyph and asks for a frame only
when `observer::any_working` says there is something to report on, and otherwise drops the
tick.

## Context

The working glyph turns every 80 ms (`labels::GLYPH_INTERVAL`, and `labels::GLYPH_FRAMES` is
the frame list, both taken from V1). Interface spec 6 draws it on working and compacting rows
only, so a server with nothing working has nothing to animate.

The alternative is a ticker the core starts when the first agent begins working and stops when
the last one finishes. That is a second piece of state about the agents, kept beside the
records that already say the same thing, and it can disagree with them. Getting it wrong in
the "off" direction freezes a working agent's glyph, which reads as a hang; getting it wrong
in the "on" direction is the cost this decision accepts anyway.

## What the always-on ticker actually costs

Measured from the code rather than assumed. `Model::agents` is a `Vec<Agent>` and `Agent.state`
is a plain `Copy` enum field, so `any_working` is

```rust
model.agents.iter().any(|a| matches!(a.state, Working | Compacting))
```

a linear scan with no allocation, no indexing, no arithmetic and no user data on the path.
There is no expression in it that can panic. At 80 ms that is 12.5 scans a second over a list
holding one entry per agent session the server knows about.

When the scan says nothing is working, `Core::animation_tick` returns without touching
`agents.glyph_tick` and without setting `view_dirty`, so no render runs and no frame is sent.

## Consequences

- The ticker is spawned in `Server::start` beside M1's one-second tick, with
  `MissedTickBehavior::Skip` so a core that stalled does not replay the frames it missed as a
  burst. `ServerHandle` holds its join handle and `stop` aborts it. The task would end on its
  own when the core's channel closes; the abort makes shutdown immediate rather than "within
  80 ms".
- The guard is one line inside `animation_tick`. Nothing in the test suite would notice if a
  later change did render work before reaching it, because an idle server's screen is
  identical either way and `ClientConn::take_frame` returns `None` for an identical frame.
  The unit tests pin that the tick is dropped; they cannot pin where the check sits.
- `GLYPH_INTERVAL` is coupled to `Harness::pump`, which returns after 50 ms of quiet. While
  anything works the core pushes a frame every interval, so an interval at or below that
  window would leave every `frame()` call reading frames until the test timed out. Lowering
  one means raising the other, and the note sits beside the constant.
- Two of the three interface tests in `tests/agents_animation.rs` are the first tests in this
  repository whose pass depends on a *rate*: `the_glyph_turns_while_an_agent_works_and_stands_still_when_it_stops`
  and `the_word_stands_still_while_the_glyph_turns`. The third,
  `two_working_agents_never_share_a_word`, waits on one deadline and reads one frame. Waiting on
  wall-clock time is not new either - `tests/agent_observer.rs` sleeps `A_TICK`, 1200 ms, to be
  sure the core's one-second tick has run - but that is a deadline elapsing, where these two need
  frames to keep arriving at an interval. The margins are wide (14 looks and 8 looks, 90 ms
  apart, each needing 3 distinct frames of 13), but a heavily loaded runner is a new class of
  flake and the failure message prints the set of frames it saw.
- `MissedTickBehavior::Skip` and the abort on stop are both untested, deliberately. Forcing a
  missed tick means a sleep in production code or a race in a test, and the consequence of
  getting it wrong is a burst of frames that self-corrects; the abort only makes an ending that
  already happens happen sooner.
