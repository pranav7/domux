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
- Mouse selection (V2.x) ends in the same Enter-to-copy path.
