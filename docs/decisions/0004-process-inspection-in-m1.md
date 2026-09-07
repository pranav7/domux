# 0004: Process inspection in M1

**Date:** 2026-09-07
**Status:** Accepted
**Decision:** `crates/domux-server/src/process.rs` holds the `ProcessInspector` trait
(`foreground(pty_fd)` through `tcgetpgrp`, `cwd_of(pid)` through libproc on macOS and
`/proc` on Linux), the real inspector and the fake. The core polls it once a second.

## Context

The interface spec (4.3) titles every pane box with its foreground command, and the
architecture spec (section 4) passes focus keys through when that command is `nvim`, `vim`
or `fzf`. Both are M1 behaviour. The agent observer (M3) is the trait's largest user, but
M1 needs the same fact earlier, and one mechanism must serve both, so the roadmap's 5.1
places the trait in M1's `process.rs`.

## Consequences

- M3's observer imports `process::ProcessInspector` instead of defining its own; the
  roadmap's 5.1 row for `agents/observer.rs` says so.
- The inspector's cwd is the fallback for a pane's working directory; OSC 7 from the shell
  wins when present.
- The fake inspector answers one value for every pane; a per-pane fake can be added when a
  test needs it.
