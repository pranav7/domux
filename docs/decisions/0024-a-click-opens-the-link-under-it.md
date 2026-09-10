# 0020: A click opens the link under it

**Date:** 2026-09-10
**Status:** Accepted. Extends decision record 0014, which stands.
**Decision:** A press and release on one cell of a pane, with no drag between them, opens the
link under that cell. A cell the program marked with OSC 8 carries its own target; otherwise
the run of text around the cell is read. Only an http or https address and a path that exists
are opened, and `domux_server::link` is the one place that decides. The opener runs as a job,
and the reader is told in a pill what was opened or what could not be.

## Context

MUX-13, with a screenshot of a Claude Code pane holding both kinds of link at once: an OSC 8
hyperlink drawn as `⧉ clonbrack-payments-experiment`, and the same address printed as plain
text on the line above it. Neither did anything when clicked. The report says they work in
Ghostty and in tmux, and asks for files and anything else that could work too.

Decision record 0014 gave the buttons to domux2 rather than to the program, so there was a
gesture to give this to. What it spent them on was focus and selection, and a click that
selected nothing did nothing at all. That click is what this uses.

## Which gesture

A click that does not move. A drag still selects and copies, which is the gesture 0014 was
written for and the one the author migrated off V1 for, so nothing about it changes. The line
between the two is the cell: the press and the release have to land on the same one, so a
press that wandered without producing a drag report opens nothing rather than opening whatever
cell it ended on.

A double click still copies the word, and its first release opens the link under it on the
way, because that release is a click. Ghostty does the same, and a reader who double clicks a
link has asked for it twice.

No modifier is needed and none is read. Ghostty opens a link on a plain click and the report
names Ghostty as the behaviour to match.

There is no underline under the pointer. That needs bare motion reporting, which 0014 left off
because nothing read it and it is the loudest mode on the wire. A link is discovered by
clicking it, which is a real cost and is recorded below rather than hidden.

## What may be opened, and why the list is short

The opener hands a string to the desktop, which chooses a program from it. The string came out
of a pipe: an OSC 8 target is whatever the program wrote, and the text of a line is whatever
scrolled past. So `link` refuses everything it does not recognise rather than passing along
what it cannot classify.

- `http://` and `https://` open as themselves.
- A path that exists opens as itself, absolute as written, `~` expanded, and relative resolved
  against the directory the shell last reported with OSC 7 or the directory the pane started
  in. It has to be on disk: a word that looks like a name is not a link.
- `file:///path` opens as the path it names. `file://otherhost/path` does not: the host is a
  machine this cannot reach, and opening it as a local path would follow a name a program
  chose to a file it named.
- Everything else is refused, `mailto:` and `javascript:` included. They are not refused
  because they are dangerous in themselves but because the list is an allowlist, which is the
  only shape that stays correct as schemes are invented.

`SystemOpener` passes `--` before the target, so an address beginning with a dash is an
argument and never a flag. That is the second of the two guards, not the only one.

## Consequences

- `Emulator` gains `hyperlink_at`, over `ghostty_grid_ref_hyperlink_uri`. Only the emulator can
  answer it: a hyperlink is a property of the cell, and the text under one is usually not the
  target, which is the point of OSC 8.
- A URL the screen wrapped opens whole. The line is read through `logical_line` and
  `text_in_range`, which rejoins the rows, and the clicked cell is found by its offset along
  the rejoined line. Long artifact addresses wrap on any real screen, so this is the common
  case rather than the careful one.
- `CoreDeps` gains `opener`. Tests pass `RecordingOpener`, which opens nothing and remembers
  what it was handed, so no test puts a browser on the author's screen.
- Opening is a `CoreJob`. It starts a process and the core task starts none (decision record
  0006). Nothing in the model changes, so the outcome is a pill rather than a reply:
  "Opened notes.md", or the reason it could not be.
- A click with no link under it says nothing. Every click answering with a line about what is
  not there would be noise on the gesture that moves focus between panes.
- A pill is drawn in the sidebar's hint row and in an overlay's footer, so a reader with the
  sidebar hidden opens a link and sees no confirmation from domux2. The application that
  opens is the answer there. Worth revisiting when the hint row has a home on the panes.

## Still open

- No hover affordance. A link is found by clicking, until bare motion is read.
- A path is opened with the desktop's own handler, so a source file opens in whatever is
  registered for its extension rather than in the reader's editor. A configured command for
  paths is the obvious next thing and is not built.
