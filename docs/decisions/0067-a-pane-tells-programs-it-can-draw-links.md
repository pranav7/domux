# 0067: A pane tells programs it can draw links

**Date:** 2026-11-26
**Status:** Accepted. Amends decision record 0016: a pane sets `FORCE_HYPERLINK` beside `TERM`
and `COLORTERM`. Makes good the link half of decision record 0044.
**Decision:** Every pane starts with `FORCE_HYPERLINK=1` in its environment, so a program that
asks before it draws an OSC 8 link draws one.

## Context

A URL Claude Code printed in its full screen view did nothing when clicked, with or without cmd.

Decision record 0044 gives the buttons to a program that asked for the mouse, and Claude Code
does. domux then opens no link for that click, on the ground that Claude Code opens its own
links. It does, but only a link it drew as an OSC 8 hyperlink. Claude Code, like most programs
that draw them, draws one only when it believes the terminal supports it, and it decides from
`TERM_PROGRAM`. Decision record 0016 drops `TERM_PROGRAM` from every pane, for good reason,
and `TERM` is `xterm-256color`. So Claude Code printed the URL as plain text, its click found no
link, and domux's own reading of the text (decision record 0024) never ran.

Cmd changes nothing: a mouse report cannot carry it, so cmd and a click reaches the program as a
plain click.

## Why this variable

`FORCE_HYPERLINK` is the name the common detection libraries read, and Claude Code reads it.
It says what is true: the emulator under every pane keeps OSC 8 links (`Emulator::hyperlink_at`),
and a click opens one, through the program under 0044 or through `link::at` under 0024.

Setting `TERM_PROGRAM` instead would name a terminal the pane is not, which is the defect 0016
removed.

## Consequences

- A link Claude Code prints opens on a plain click, the way 0044 said it would.
- A program in a pane that asked for nothing now draws OSC 8 links too, and a click on one opens
  its marked target through `link::at`'s allowlist, as 0024 already provides.
- A pane picks the variable up when it starts. A pane already running, including one carried
  across `domux server upgrade`, keeps the environment it started with.
