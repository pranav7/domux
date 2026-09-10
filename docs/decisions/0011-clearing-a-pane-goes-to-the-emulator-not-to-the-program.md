# 0011: Clearing a pane goes to the emulator, not to the program

**Date:** 2026-09-09
**Status:** Accepted
**Decision:** `pane.clear`, `domux2 pane clear` and `leader k` empty a pane by feeding
`ESC [ H ESC [ 2 J ESC [ 3 J` to that pane's own emulator. Nothing is written to the pane's
program. The alternate screen is refused, and copy mode ends because the history it walks is
what the clear takes away.

## Context

MUX-7's sibling report, MUX-5, said clear does not fully clear the workpanel, with a
screenshot of a pane full of rubbish and `clear` typed at the bottom of it.

Measured against this build, `clear` typed in a pane does clear it, screen and scrollback
both. What the screenshot shows is something else, and the pane's own emulator identifies it:
asked for its primary device attributes it answers `ESC [ ? 62;1;2;6;22 c`, which is exactly
the text repeating across those lines. A binary file printed to a terminal carries `ESC [ c`
bytes in it; the emulator answers every one of them into the pty; the shell, sitting at its
prompt, reads the answers as typed input and echoes them. Every terminal behaves this way, and
it is why printing a binary file wrecks a prompt anywhere. The `clear` in the screenshot never
ran, because what the line editor held was not what was typed.

That is the case worth having a key for, and it decides where the bytes go. A key that typed
`clear` at the program, or sent it a form feed, would be asking the program to act at the one
moment the program is the thing that is broken. So the bytes go to the emulator, and the pane
comes back empty whatever its program is doing with its input.

## Consequences

- The shell does not know its screen was cleared, exactly as it does not know when `clear`
  runs. Its next prompt draws at the top, because the cursor went home with the erase.
- Rubbish already in the shell's line editor survives, because this key never touches the
  program's input. `C-c` is what clears that, and it always was.
- A full screen program is refused rather than cleared: there is no scrollback to erase on the
  alternate screen, and the program would not know to redraw, leaving a blank pane over a live
  editor.
- `leader k` was free. `k` is `list.up` inside the Projects box, which is a different keymap
  and unaffected.
