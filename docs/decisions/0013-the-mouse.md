# 0013: The mouse

**Date:** 2026-09-09
**Status:** Accepted
**Decision:** domux2 reads the mouse. The wheel over a pane belongs to that pane's program
when the program asked for mouse events, and to copy mode otherwise. The buttons are always
domux2's: dragging inside a pane selects text and copies it on release, a click focuses the
pane under it, a double click selects a word and a triple click selects a line. A click on the
chrome runs the same handler the key for that operation runs.

## Context

Two reports, one cause each.

Scrolling does nothing in a pane running Claude Code. Claude Code enters the alternate screen
and turns mouse tracking on (`ESC [ ? 1049 h`, then `1000 h`, `1002 h`, `1003 h`, `1006 h`)
as its interface starts, verified by capturing its output on a pty at six screen sizes and
five values of `TERM_PROGRAM`. It scrolls its own transcript when it is sent a wheel report:
sent `ESC [ < 65;5;5 M` it answers with a 762 byte redraw. It has a `/scroll-speed` command,
and it tells tmux users to set `mouse on` so the wheel reaches it. So the pane has no
scrollback of its own to walk, on purpose, and the only thing that can scroll it is the
program.

domux2's client already reports the wheel, and `Core::scroll` offered the gesture to copy mode
alone. Copy mode has nothing to walk on the alternate screen, by decision 0005 and for the
reason recorded there, so it refused, and nothing else in the server ever wrote a mouse report
to a pane. The gesture was dropped. In a pane on the primary screen the same gesture reaches a
real scrollback, which is why the wheel looked like it worked everywhere else.

Text cannot be selected with the pointer. The client turns mouse reporting on so it can learn
the wheel's position, and while that is on the outer terminal hands presses and drags to
domux2 rather than making its own selection. The client discarded everything except the wheel,
and nothing in domux2 put a selection in their place. `shift` and drag still reaches Ghostty's
own selection, because `mouse-shift-capture` defaults to false, but that is an escape hatch
and not an answer.

Decision 0005 called mouse selection V2.x work. It is a blocker for the author's migration off
V1, so it is M2 work now, and the rest of the pointer comes with it: a click on a tab, on the
`+`, or on a row of the sidebar does what the key for it does.

## The rule about buttons

The wheel is the program's when the program asked for the mouse. The buttons are not: they
select text and move focus, whatever the program asked for. A program that wants the buttons
does not get them in this milestone.

This is the line that keeps one selection. If the buttons went to Claude Code, selecting its
text with the pointer would stop working the moment it turned mouse tracking on, which is the
report this decision answers. The cost is that a full screen program's own clickable interface
does not respond to a click, and the wheel is the one gesture it does receive.

## Ghostty's selection engine, and why the selection is not built on it

Ghostty's C API carries a whole selection subsystem: `ghostty_selection_gesture_*` tracks a
press, drag and release with a click count, offers cell, word, line and command-output
behaviours, and formats a selection with soft-wrapped lines rejoined the way Ghostty's own
copy does. It is a better engine than anything written here would be.

It is not used, because copy mode already holds a selection: an anchor and a cursor in
scrollback coordinates, drawn in reverse video by `render::pane_box`, read by `text_in_range`
and copied by the path Enter takes. Binding the gesture engine would put a second selection
model beside that one, in pixel geometry rather than cells, and the pointer and the keys would
then disagree about what is selected and what copying it means. One selection, reached two
ways, is worth more than a better engine reached one way. What the pointer needs beyond copy
mode is a word boundary and a line extent, and those are small.

If the keyboard's copy mode is ever rebuilt on the gesture engine, the pointer follows it
there, in that order.

## What one cell belongs to

`render::hit_at` answers it, from the same `RenderInput` the frame is drawn from, and every arm
asks the module that draws that region: the tab row measures its own pieces, the Projects box
finds its own row, the pane boxes come from the layout the renderer solves. Nothing on the
pointer's side measures the screen a second time, because a second measurement is how the cell a
reader clicks stops being the cell they see.

Three things fell out of that and are worth naming:

- `TabRow` is laid out once into pieces and then drawn from them, rather than measuring as it
  draws. The elision, the `+` and the cut anchor cell are placed in one walk that both the
  drawing and the pointer read.
- The Projects box's rows, and the rule that its filter applies only while it has the keys, moved
  into one function the drawing and the pointer share.
- `top_bar::share` is the one place the tab row and the right end divide their cells.

## Consequences

- `ClientMsg` gains `Mouse`, and `PROTOCOL_VERSION` goes to 3. A client and server that
  disagree already refuse each other with an instruction, so a running server has to be
  restarted for this.
- The message carries a click count. The client counts repeated presses, for the reason it turns
  one wheel notch into a fixed number of lines: the timing is the outer terminal's. It also keeps
  the server a function of the messages it was sent, so a double click is a test rather than two
  presses and a sleep.
- The client enables mode 1002 as well as 1000, so a drag arrives. Bare motion (1003) stays
  off: nothing in domux reads it, and it is the loudest mode on the wire.
- `Emulator` gains `Mode::MouseTracking` and `encode_mouse`, beside `encode_key` and
  `encode_focus`. The report's shape is the program's business, so it comes from Ghostty's
  mouse encoder configured from that pane's own terminal state. The encoder works in pixels; one
  cell is one pixel, which makes the mapping an identity rather than a second geometry.
- `Emulator` also gains `logical_line`, which is what a triple click selects. Whether a row
  continues the one above it is a soft-wrap flag on the row, so only the emulator can answer it.
- A pane whose program asked for the mouse can no longer be scrolled into copy mode with the
  wheel. `leader [` still opens copy mode there, and on the alternate screen it walks the
  screen it can see.
- One wheel gesture is one report to the program, whatever line count the client sent. The line
  count is copy mode's step, and the protocol the program reads carries no magnitude either.
- A drag that selects nothing copies nothing and says so, by the path decision 0005 already
  built for Enter. Enter and a release both copy through `copy_mode::yank`.
- Clicks on the chrome dispatch `Method`s, so a click, a key and a CLI subcommand stay one
  handler: a tab's cell is `tab.select`, the `+` is `tab.create`, a workspace's row is
  `workspace.focus`. A refusal reaches the hint row the way a key's refusal does.
- A press acts on the chrome and a release does not, so an operation runs once per click.
- The middle and right buttons are reported and do nothing. A middle-click paste would put the
  primary selection somewhere the reader cannot see it first.
- A full screen program's own clickable interface does not respond to a click. That is the cost
  of the rule about buttons above.
- "No mouse" in the repository guidelines is now wrong and is replaced by this decision.

## Still open

- The switcher's rows do not answer a click. An open overlay owns the screen, and the switcher is
  the one overlay whose rows would mean something; it wants the same treatment the sidebar's box
  got here.
- Dragging past a pane's own rows does not scroll the viewport, so a selection cannot run into
  the scrollback with the pointer alone. Ghostty's gesture engine has autoscroll for this, and
  copy mode's keys can already do it.
- A drag that starts in one pane and ends in another selects in the first, clamped at its edge.
