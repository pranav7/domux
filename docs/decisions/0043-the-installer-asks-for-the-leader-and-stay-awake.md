# 0043: The installer asks for the leader and for stay awake

**Date:** 2026-09-13
**Status:** Accepted. Amends decision 0039 (what the installer writes, and that nothing in it
moves).
**Decision:** Once the binary and the hooks are in, the installer asks two questions and writes
the answers to the config file: "Select your leader", a pick from `C-s`, `C-a`, `C-b` and
`C-Space` or a typed key name, and whether to set up stay awake for a closed lid, on macOS and
Linux both. It never replaces a leader or a stay awake mode the file already sets. A finished
step wears a check mark and says in plain words what was detected and what was done, and the
release lookup and the download turn a spinner while they wait.

## Context

MUX-35. The author installed 1.0.0 on Linux and the installer asked nothing. The stay awake
question was macOS only, the leader was left to the binary's default, and the last step read
"hooks for claude codex opencode", which did not say whether those agents had been found, had
their hooks installed, or were about to. Every step wore a faint dot, finished or not.

The same issue moved the default leader from `C-a` to `C-s` in the binary. `install.sh` is
served from `main` and installs whichever release it is given, and 1.0.0's default is still
`C-a`, so the installer cannot describe the leader by leaving it alone.

## The leader question

The leader is the first key a reader of a multiplexer has an opinion about, and a wrong one is
felt on the first keypress. The people who install domux know what a leader is, so the question
is three words and a short list: `C-s`, the default; `C-a`, screen's and V1's; `C-b`, tmux's;
and `C-Space`. A fifth choice takes a typed key name. One keypress picks, a number or the key
itself, and enter takes the default.

A key is read as one byte with the terminal's line editing, echo and flow control off. Flow
control has to be off because `C-s` is XOFF: with it on, the terminal keeps the key and stops all
output, so pressing the default leader at its own question froze the install until `C-q`. It
stays off from the first question until the script exits, when the trap gives the terminal back
the settings it had. Turning it back on after each key was not enough: a reader who presses `C-s`
twice, or holds it, sends a second one while the installer writes the leader, and that one
stopped the output before the next question could turn flow control off again. The byte goes
through `od`, because `C-Space` sends a zero byte and a shell drops one. Keys typed before a
question are thrown away, so an enter pressed while the download turned does not answer it, and
so is the rest of a key that sends several bytes, such as an arrow, which the y or n question
ignores rather than taking as no. A key that is not on the list is not an answer.

A typed name is checked the way `[keys]` reads one, modifiers and then one character, a named key
or F1 to F12, and asked again until it is one. A name with a control character in it is refused
too, because TOML refuses one inside a string, and so is a character that is not UTF-8, because a
TOML file is UTF-8 and one such byte makes domux refuse the whole file. The check reads the
character's bytes through `od` rather than matching it with `?`, because dash, and any shell in
the C locale, match `?` against one byte: there the check refused `C-é` and took a stray byte
that is not a character at all. The binary treats a bad leader as an error, because nothing works
without one, so the installer never writes a leader domux would refuse.

`DOMUX_LEADER` answers the question without asking and is checked before any request, so a bad
value fails before anything is downloaded.

The answer is written even when it is the default. The line says which key the reader has, and
writing it makes that true for every release the script installs, 1.0.0 included.

With no terminal to ask on, the question takes its default, the same as a reader pressing enter,
and a note under the step says so. The stay awake question follows the same rule, and its
default writes nothing.

A leader the config file already sets is kept, and the step names it. `DOMUX_LEADER` does not
replace it either, and a note says so when the two differ. The file is the reader's, and a
reinstall is an upgrade, not a reset.

## The stay awake question

The question is asked on every system, because a closed lid on a Linux laptop sleeps the same as
one on a Mac. A yes writes `mode = "full"` under `[stay_awake]`. On Linux that is all it takes:
full mode is one more word for the child that holds the machine awake, and no sudo is involved.
On macOS the installer first runs `stay-awake install --full --apply`, which asks for sudo once,
and writes the line only when that worked. A full mode without the launch daemon would be a mode
the machine cannot hold. The installer used to print the line for the reader to add by hand;
writing it is the step a reader was most likely to skip.

Stay awake itself is a toggle held in the state, off until someone turns it on; the mode only
says what kind of hold it is. So the step after a yes says how to turn it on, naming the leader
the reader just chose: "press C-s then A inside domux to turn stay awake on or off". A no, or no
terminal, writes nothing and says how to set it up later. A mode the file already sets is kept.

A domux that is already running read the config file when it started, and keeps that leader until
it reads the file again. So when the install replaced a domux binary and wrote either line, a note
says to run `domux config reload`. Whether a server is running is not checked, because asking one
means connecting to it; a binary already at the install path is the sign that one may be.

## How the config file is written

Every write in domux goes to `path.tmp` and is renamed over the file, and a file the reader owns
asks for more: keep every line, add only the line that is missing, keep the file's permissions,
and write through a link to the file it points at, so a config file kept in a dotfiles
repository stays a link.

The edit is an awk program inside the script, and the script checks for awk with the other
tools it needs before it starts. The other ways were worse:

- **Appending.** A file that already has a `[keys]` table would get a second one, and TOML
  refuses a table defined twice. The leader has to go under the header that is there.
- **sed.** GNU and BSD sed disagree on `-i` and on the syntax for adding a line after a match,
  and sed reads a line at a time, so it cannot easily know which table a line is in.
- **The binary.** There is no `config set` command, and 1.0.0, which this script still installs,
  will never have one.

awk is on every machine the script runs on and reads the whole file the same way on each. The
program knows table headers, bare or quoted, keys inside a table and dotted keys before the first
header, bare or quoted, a byte order mark at the start of the file, and values in one-line and
multi-line strings. It follows multi-line arrays and strings so a line inside one that looks like
a header is not read as one. A file that sets `keys` or `stay_awake` without a header, as an
inline table, with dotted keys, or as an array of tables, is left alone, because a header added
under it would define the table twice; the step says what to add by hand. A leftover `path.tmp`
is removed before the write, so a link there is never written through.

It is not a TOML parser, and a parser reads forms it does not. A table or key name spelled with
an escape, such as `"le\u0061der"`, is not recognized, so the installer would add a second one
and domux would refuse the file. A value with a `\u` escape in it is shown wrongly, which changes
only the step's line, since a value that is there is never replaced. Reading the file back after
the write would not catch any of this, because the same program would read it both times, so
each form the first version missed is handled where the file is read instead.

The suite passes under gawk, under mawk, which is Ubuntu's awk, under the awk macOS ships and
under busybox awk. A set of config files that covers every form above, plus comments, CRLF line
ends, brackets and quotes inside strings and a missing last newline, was run through the program
under all four, with the same bytes out of each, and checked with `Config::parse` before and
after: every file that parsed before parsed after, with the leader and mode it set or the ones
written. That is a sample of the forms people write, not a proof.

## The steps

A check mark replaces the faint dot on a finished step. A cross marks a step that did not work
while the install carries on, such as a hooks install that failed, because a check mark there
would say it worked. The red dot stays on a question, because it means domux is waiting on you.

Each step says what was detected and what was done: the system and architecture detected, the
release found or pinned, the size downloaded and the checksum verified, where the binary went,
one line per coding agent detected with its hooks installed, a line when no agent is detected,
the leader, stay awake, and then "domux is ready" and "> run domux".

The release lookup and the download turn the braille spinner while they wait, and nothing else
does; decision 0039 says why it came back. A request runs in the background while the frames
turn. The script looks at the request every 40 ms rather than sleeping a quarter second before
it looks, because a sleep there cost every request a quarter second, three quarters of a second
on every install, even when GitHub answered at once. The first frame waits for the sixth look, a
quarter second, so a request that answers sooner draws nothing, then a frame turns every second
look, every 80 ms, and the last one stays until the step's check mark replaces it, so the two
downloads read as one step. A request that answers just after the quarter second draws one frame
and loses it to the check mark within a look or two. Any delay has that edge, and holding the
frame longer would put back the wait the looks took out. The cursor is hidden while a frame
turns. A trap on exit,
INT and TERM kills the request the spinner started and nothing else, clears the line, shows the
cursor, and gives the terminal back the settings a question changed.

`NO_COLOR` takes away the color and the spinner, not the questions: a reader who wants no color
is still at the terminal. What decides whether to ask is whether stderr is a terminal and the
terminal can be opened, so a log file never gets a question nobody can see.

## Consequences

- The installer writes the config file. A machine installed without a terminal ends up with
  `leader = "C-s"` in it. So does a machine that relied on 1.0.0's `C-a` and re-runs the
  installer without a terminal, or presses enter at the question; the changelog says how to keep
  `C-a`.
- Pressed twice, the leader sends `C-s` to the pane. A program that reads keys itself, such as an
  editor, gets it, but a shell at its prompt usually leaves flow control on, and there it pauses
  the pane's output until `C-q`.
- A machine that still keeps its config file under the name used before the cut-over of
  2026-09-11, and has not started a server since, gets a new config file from the installer. The
  server moves the old file only when no new one exists, so the old one is then left where it is
  and ignored. Only machines that ran domux before its first release can be in that state, and
  the installer does not spell the old name, so this is recorded rather than handled: move the old
  file into place and run the installer again.
- `C-s` is the default in two places, `KeysConfig::default` and `LEADER_DEFAULT` in
  `install.sh`, and they move together.
- `tests/install/run.sh` runs the spinner, the questions and the signals on a terminal through
  util-linux `script`. BSD `script` takes other flags, so those tests skip on macOS and run on
  the Ubuntu job. A job a script starts in the background begins with SIGINT ignored, and a
  shell cannot trap a signal it started out ignoring, so the harness starts the installer with
  GNU `env --default-signal=INT`.
- The fake `sudo` in `tests/install/fakebin` refuses and records the call, so no test can raise
  privileges whatever the installer or the binary it installs goes on to run.
