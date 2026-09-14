# 0044: A program that asked for the mouse gets the buttons

**Date:** 2026-09-14
**Status:** Accepted. Replaces the rule about buttons in decision record 0014, and narrows
decision record 0024 to panes whose program did not ask for the mouse. The rest of both stands.
**Decision:** The buttons go where the wheel goes. A press over a pane whose program asked for
mouse events is reported to that program, whichever button it is, and so are the drag and the
release that follow it, wherever the pointer is by then. The press still focuses the pane.
domux selects nothing, copies nothing and opens no link for that gesture. A pane in copy mode
keeps the buttons for domux, and so does a pane whose program asked for nothing.

## Context

MUX-33, with a screenshot of a Claude Code pane. The diff sidebar Claude Code draws on the right
of its screen has a `×` in its corner, and a click on it did nothing.

Decision 0014 kept the buttons back on purpose. Its reason was one selection: if the buttons
went to Claude Code, selecting its text with the pointer would stop working the moment it turned
mouse tracking on. The cost it accepted was that a full screen program's own clickable interface
does not respond to a click. MUX-33 is that cost, reported.

The reason no longer holds. Claude Code 2.1.270, the version on the author's machine, answers
the buttons itself, and its binary says how:

- It selects its own text with a drag, draws the selection, and copies it when the drag ends.
  `copyOnSelect` is a setting, and it is on unless the reader turns it off.
- It copies with the desktop's own tool when it is not over SSH (`pbcopy` on macOS, and
  `wl-copy`, `xclip` or `xsel` on Linux) and writes OSC 52 as well. domux reads no OSC 52 from
  a pane, so the desktop's tool is the path that reaches the clipboard.
- It opens a hyperlink it drew when that link is clicked.
- A click moves the cursor in its prompt and expands a collapsed tool result, and its diff
  sidebar has a `×` that closes it. `CLAUDE_CODE_DISABLE_MOUSE_CLICKS` turns clicks off, and
  its hints tell the reader to hold shift or option to reach the terminal's own selection.

So Claude Code with the buttons has everything 0014 gave the reader, drawn by the program that
knows what its own text is, and Claude Code without them has a clickable interface that does
nothing. Ghostty, and tmux with `mouse on`, already hand the buttons to a program that asked for
them, which is what Claude Code was written against.

## Which gesture belongs to whom

The press decides, and its decision lasts until the release.

- A press over a pane is the program's when the program has mouse tracking on (DECSET 9, 1000,
  1002 or 1003) and the pane is not in copy mode. It is reported at the pane's own cell, it
  focuses the pane, and it clears unseen on the pane's agent, because it is input reaching that
  pane in the same sense a key is.
- The drag and the release after that press go to the same program, whatever is under the
  pointer. A cell outside the pane's box is reported at the nearest cell inside it, the way a
  cell on the box's rule already was. Claude Code copies a selection when its drag ends, so a
  release over the pane beside it that went unreported would leave a selection that is never
  copied and a button the program believes is still down.
- The client remembers where its press went (`ClientConn::reported_press`) rather than the
  pane, because the question a release asks is which program this client's press went to, and
  the pane under the pointer at the release does not answer it.
- A press domux took stays domux's to its end, even when the program turns tracking on before
  the release.
- Which of these events the program hears is its own choice. Ghostty's mouse encoder is set
  from the pane's terminal state, so a program that asked for presses alone hears no drag.

## Why copy mode keeps the buttons

A pane in copy mode shows copy mode's viewport, drawn by domux over the program's screen, and
the copy cursor and the selection there are domux's. A press on that viewport is about what the
reader sees, which is copy mode. So `leader [` and a drag is how text leaves a program's pane
through domux's own selection, by the same copy path Enter takes.

The wheel is unchanged, in copy mode as everywhere else.

## Consequences

- Clicks in Claude Code work: the `×` closes the sidebar, a click moves the cursor or expands a
  result, and a drag selects and copies with Claude Code's own selection.
- domux's drag, double click, triple click and link click no longer act over a program that
  asked for the mouse. That covers every full screen program with mouse support, not Claude
  Code alone, and each of them treats the buttons the way it does in any other terminal. Shift
  and a drag still reach Ghostty's own selection, because `mouse-shift-capture` defaults to
  false.
- A program that asks for the mouse and has no selection of its own leaves the pointer nothing
  to select with there. `leader [` is the way in.
- Claude Code opens its own links, so a link there opens the way Claude Code chooses rather than
  through `link::at`'s allowlist.
- The middle and right buttons reach a program that asked for the mouse. Over any other pane
  they still do nothing.
- `render::cell_in_pane` answers where a screen cell lands in a given pane's grid, beside
  `render::hit_at`, and both are asked through the same `RenderInput` the frame is drawn from.
- The protocol does not change. The client already reported every button, each drag and each
  release.

## Still open

- domux reads no OSC 52 from a pane. A program that copies only that way, such as Claude Code
  over SSH, copies nothing through domux.
- Bare motion is still not reported, so a program's hover effects do not show.
- No modifier takes the buttons back for domux over a program that asked for the mouse. Shift
  is Ghostty's, and no report has asked for another.
