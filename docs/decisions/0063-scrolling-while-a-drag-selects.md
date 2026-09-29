# 0063: Scrolling while a drag selects

**Date:** 2026-09-29
**Status:** Accepted. MUX-56. Closes the gap decision record 0014 left open about a drag past a
pane's edge, and adds a case to 0014's rule about the wheel. The rest of 0014 and 0044 stands.
**Decision:** A drag domux is selecting with keeps its pane from the press to the release,
wherever the pointer goes, and held past the pane's top or bottom edge it scrolls the pane one
line a tick. The wheel over a program on the alternate screen that asked for no mouse is Up and
Down keys, as Ghostty sends it, unless the program turned mode 1007 off.

## Context

MUX-56: "I was selecting a long text that spanned multiple scroll pages, and I couldn't scroll
it inside codex and claude."

Two things were behind it, one for each program.

**Claude Code** runs on the primary screen unless its full screen mode is on. Version 2.1.285
asks for no mouse there (it sets bracketed paste, 2031 and focus reports, and nothing else), so
the drag is domux's. The wheel during that drag already moved copy mode's viewport and the
selection grew with it. What did not work was the gesture every terminal teaches: dragging to the
edge of the text and holding there. A drag clamped to the pane's edge and never scrolled, and a
drag that left the pane for the chrome or the pane beside it was dropped, so a release there
left copy mode open with a selection nobody copied. 0014 recorded the first as open.

In its full screen mode Claude Code asks for the mouse, takes the buttons by 0044, and scrolls
its own selection: it autoscrolls while the drag is below its transcript, and the wheel reaches
it. Nothing here changes that.

**Codex** 0.159.0 enters the alternate screen, turns mouse tracking off and sets mode 1007,
alternate scroll, so that the terminal sends Up and Down for the wheel and its transcript
scrolls. domux turns alternate scroll off on the outer terminal, so a gesture it did not report
cannot become keys (the client's terminal module), and it offered the wheel to copy mode, which
has no history to walk on the alternate screen. The wheel over Codex did nothing at all.

## A drag belongs to the pane it started in

`ClientConn::selecting` holds the pane a press from this client started a selection in, beside
`reported_press`, which holds the pane a program took the press for. The drag and the release go
to that pane at its nearest cell, the way 0044 sends a program's drag to its program's pane. So a
drag onto the tab row puts the copy cursor on the pane's top row, and a release there copies.

A release off the pane that never dragged is not a click on the cell it clamps to, so it opens
no link.

## The edge scrolls on the ticker

While the pointer is above the pane the drag scrolls it into the history, and while it is below
the pane back towards the live screen, one line at a time. The step that crosses the edge moves
at once and the animation ticker, 70 ms, takes each step after it, so the pane keeps moving while
the pointer stays still. That is about fourteen lines a second, and it needs no timer of its own:
the ticker already never stops (0019, 0032). The copy cursor stays on the edge row, so each line that scrolls in joins the
selection.

The outer terminal reports a pointer that leaves its own screen at the screen's edge, so above
the pane means the top bar or the tab row, and below it the rows under the workpanel. Distance
does not change the rate: there are only a row or two to move into.

## The wheel is keys on the alternate screen

Ghostty sends Up or Down, one per line, in the cursor key mode the program set, when three things
hold: the alternate screen is up, the program asked for no mouse, and mode 1007 is on. Mode 1007
is on until a program turns it off. domux now does the same per pane, from the pane's own state,
with the same line count copy mode takes, so Codex scrolls, and so do `less` and other full
screen programs that leave the mouse alone.

Ghostty drops its selection when it sends those keys, because the program is about to redraw the
cells it covers. domux does the same with its own: the wheel leaves copy mode on that pane and
forgets the press. A selection across Codex's pages is not something any terminal can make on the
alternate screen. `codex --no-alt-screen` runs Codex on the primary screen, where its transcript
is scrollback and the drag and the wheel reach into it.

## Consequences

- A selection made with the pointer alone reaches into the scrollback.
- A drag that wanders onto the chrome or into the pane beside it still moves its selection,
  clamped to its own pane, and a release anywhere copies it.
- The wheel over a full screen program that asked for no mouse sends it keys it did not get
  before. A program that does not want them turns 1007 off, as it would for Ghostty.
- `Mode::AlternateScroll` joins the `Emulator`'s modes.
- The client still turns alternate scroll off on the outer terminal. That stops the outer
  terminal making keys for domux's own screen; the pane's mode is a separate question.
