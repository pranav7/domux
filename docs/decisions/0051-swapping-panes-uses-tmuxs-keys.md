# 0051: Swapping panes uses tmux's keys

**Date:** 2026-09-15
**Status:** Accepted
**Decision:** `pane.swap <dir>` trades a pane's place with a neighbour, where `dir` is
`previous`, `next`, `left`, `right`, `up` or `down`. `leader {` binds `pane.swap previous` and
`leader }` binds `pane.swap next`, which are tmux's keys for `swap-pane -U` and `swap-pane -D`.
The four directions have no default key. What a swap does to focus and to a zoom is what tmux's
`swap-pane` does with neither `-d` nor `-Z`.

## Context

MUX-47 asks for panes that swap up, down, left and right. The design principle the author holds
domux to is that a tmux user feels at home, so a default key is the key tmux gives the same
action.

tmux binds two swap keys by default, `prefix {` and `prefix }`, which step through the panes in
order. It has no default key for a swap in a direction; `swap-pane -t '{left-of}'` exists but
nothing binds it. The author was offered three choices on 2026-09-15: tmux's two keys alone,
those plus `leader H/J/K/L` after vim's `C-w H/J/K/L`, or those plus `leader M-arrows`. The
author chose tmux's two keys alone.

## What a swap does

- **The two leaves trade places and every split stays.** Each pane takes the other's box at the
  other's size, and its program is told the new size. Rebuilding the splits instead would move
  boxes the reader did not ask to move.
- **Focus goes with the pane that was named.** tmux makes the target of `swap-pane` the active
  pane, so pressing `leader }` again and again walks the focused pane along the row. When a script
  names a pane that is not focused, that pane is focused afterwards, as `pane.zoom` already does.
- **A zoom is cleared and reported** with `pane.zoomed`, as tmux does without `-Z` and as
  `pane.split` does. A direction is measured on the whole layout, since a zoom hides every
  neighbour.
- **`previous` and `next` wrap** at the ends of reading order, as `-U` and `-D` do.
- **A direction names the pane a focus move would reach**, through `neighbour_by_geometry`, so
  `pane.swap left` and `C-h` never disagree about which pane is on the left.
- **With nothing to swap with, nothing changes**, the zoom included, and the call succeeds with
  `with: null`. A focus move at an edge does nothing and does not fail, and a swap at the same edge
  behaves the same way. A script reads `with` to learn whether a swap happened.

## The event

`pane.swapped` names the tab and both panes. Two panes of one size that swap resize nothing, so
without it the event stream says nothing when the layout changed.

## Consequences

- The keys overlay has two more rows under `workpanel`.
- `leader H`, `J`, `K` and `L` stay free. A reader who wants the directions binds them in
  `domux.toml`:

  ```toml
  [keys.bindings]
  "H" = "pane.swap left"
  "J" = "pane.swap down"
  "K" = "pane.swap up"
  "L" = "pane.swap right"
  ```

- `domux pane swap <dir>` swaps the pane `DOMUX_PANE` names, and prints nothing.
