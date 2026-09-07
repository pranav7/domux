# 0003: Detach and help keys

**Date:** 2026-09-07
**Status:** Accepted (plan assumption; the author may re-rule)
**Decision:** `client.detach` is an API method and `leader d` binds it. `help` is a view
method that opens the keys overlay and `leader ?` binds it. Both are in the default keymap.

## Context

The specs name no detach key or method, and a multiplexer that cannot detach cannot meet
the M1 exit criterion (detach and reattach in one day). Design principle 3 asks for a place
where every configured binding shows; the interface spec binds `?` only inside the Projects
and Agents boxes, which arrive with M2 and M3. `leader d` is tmux's detach key and `?` is
tmux's key list, so the author's fingers already know both.

## Consequences

- `leader d` and `leader ?` are ordinary bindings: a user may move them in `domux.toml`,
  and the chord indicator's `? keys` hint follows the configured key.
- The help overlay renders from the loaded keymap, never from a hard-coded table.
- M2 adds `"s"` and `"b"`, M3 `"a"`, M4 `"o"` and `"U"` to the same default table.
