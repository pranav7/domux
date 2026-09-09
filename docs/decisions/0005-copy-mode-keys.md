# 0005: Copy mode keys and appearance

**Date:** 2026-09-07
**Status:** Accepted (plan assumption; the author may re-rule)
**Decision:** Copy mode keys are fixed in M1: `h j k l` and the arrows, `0` and `$`, `g` and
`G`, `C-u` and `C-d`, `PageUp` and `PageDown`, `v` to start a selection, Enter to copy and
leave, Esc or `q` to leave. The pane's box stays; its flag reads `copy`, or `copy N/M` once
scrolled N lines above the bottom of an M-line scrollback. The clock's place shows
`v select · ⏎ copy · esc leave`. The selection is drawn in reverse video.

## Context

The architecture spec fixes the model (movement, `v`, Enter, Esc) and the interface spec
leaves the appearance to this plan, asking only that the mode be visible in the border
(principle 1). The keys are vi's, which the author's tmux uses. Reverse video reads without
colour (principle 2). Enter with no selection leaves without copying, as in tmux.

## Consequences

- The keys are not configurable in M1. If a `[keys.copy]` table is wanted, it is one
  more table in `KeysConfig` and one lookup in `copy_mode::handle_key`.
- Mouse selection ends in the same copy path. It arrived in M2 rather than in V2.x: see
  decision 0013.

## Extension, 2026-09-08

A vertical wheel gesture over a pane enters copy mode and moves its viewport immediately.
The client enables only the terminal modes needed to learn the wheel's screen position. It
ignores clicks, drag, motion and horizontal scroll. Mouse selection remains V2.x work.

## Extension, 2026-09-09

Superseded in part by decision 0013. The client now reports the buttons and drag as well as the
wheel, a drag selects and copies on release, and the wheel over a pane whose program asked for
the mouse goes to that program instead of to copy mode. The keys above are unchanged, and both
the pointer and Enter copy through `copy_mode::yank`.
