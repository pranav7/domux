# 0066: Ghostty hands cmd and a number to domux

**Date:** 2026-11-26
**Status:** Accepted. Amends decision record 0065: the client reads cmd only where it knows cmd
and a number reaches domux. The rest of 0065 stands.
**Decision:** `domux install ghostty` adds the lines that unbind cmd and a number to the last
config file Ghostty reads, and asks Ghostty to confirm. At attach, a client in Ghostty on the
reader's own Mac asks Ghostty whether it still keeps the chord, and reads cmd only when it does
not. When it does, the hint row names the install until the reader's next key.

## Context

0065 hid the agents' numbers until cmd is held at any client on a Mac. That holds up only where
cmd and a number reaches domux. In Ghostty with its default keys, a reader held cmd, saw `2` next
to an agent, pressed cmd and 2, and Ghostty switched its own tab. The author asked for a setup
step that does the unbinding and confirms it.

The first attempt at the unbinding missed half of it. Ghostty binds each number twice: by the
character the key types, `super+1`, and by the key itself, `super+digit_1`. Unbinding `cmd+1`
removes the first and leaves the second, and the second still takes the chord. So the lines are
both forms, eighteen in all, and only Ghostty can say whether they worked.

## Asking Ghostty

`ghostty +list-keybinds` prints the bindings left once every config file Ghostty reads is
loaded, in the order it reads them. Its answer is the one that counts, whatever file a binding
came from. `ghostty::kept_digits` reads the numbers from it that cmd alone still takes, a
sequence that starts with the chord included, and nothing else. A binding with another modifier,
such as cmd and shift and 1, is another chord and does not count.

The binary is in `GHOSTTY_BIN_DIR`, which Ghostty sets in the shells it starts, then on `PATH`,
then in the app under `/Applications`. A Ghostty that cannot be run says nothing either way.

The answer is about the files, not about the Ghostty that is running: a change takes effect when
Ghostty reloads its config. The install says so, and the attach check runs at every attach, so
the next attach after a reload agrees with it.

## The install

`domux install ghostty` has the shape of the agents' installs: it previews by default, and
`--apply` backs up the file, writes `path.tmp` and renames it, and changes nothing on a second
run. It writes nothing when Ghostty keeps no number.

Ghostty reads `config` and `config.ghostty` under `$XDG_CONFIG_HOME/ghostty`, and on macOS the
same two under Application Support after them, and a later file wins. The install writes the last
of these that exists, so the lines come after every binding Ghostty read before them, and it
creates the last of the list, Ghostty's own choice, when none exists. It writes through a link to
the file the link names, so a config kept in a dotfiles repository is the one written and the
link stays a link.

After writing it asks Ghostty again. It says the keys are domux's only when Ghostty agrees, and
otherwise fails with the chords still kept, which is what a file read later that binds them again
looks like.

The command is `install` because it does what the other installs do: it patches another
program's configuration so that program works with domux, and it never runs without being asked.
`--dir` is refused for Ghostty, which finds its files by its own rules.

## The check at attach

The client reads cmd only when all of these hold: it runs on a Mac, not over ssh, in Ghostty
(`TERM_PROGRAM`), and Ghostty keeps no number with cmd. In every other case it says nothing about
cmd and the Navigator writes the numbers all the time, as 0064 drew them, so a number on the
screen is always one a key reaches. Other terminals keep cmd and a number for their own tabs too,
and none answers the question Ghostty does.

Asking Ghostty takes about a tenth of a second, so the check runs on a blocking thread beside the
session and the session starts at once. Until it finishes the client reads nothing, which is the
same as a client that cannot tell.

When Ghostty keeps the chord, the client sends `ClientMsg::Notice` with `Ghostty keeps cmd+1-9:
run domux install ghostty --apply`, and the server shows it as an action hint, which the reader's
next key clears. It is sent at every attach, including the one after an upgrade, until the keys
are handed over. `Notice` carries text because the client is the one that knows, and it may say
other things the client alone can see. It is not the reader's activity. `PROTOCOL_VERSION` is 6.

## Consequences

- A Mac user in Ghostty with its default keys sees the numbers all the time and a hint at
  attach, until they run `domux install ghostty --apply` and reload Ghostty's config.
- A Mac user in another terminal sees the numbers all the time.
- `install ghostty --apply` writes a file outside domux's own, so, like the agents' installs, it
  is never run against the author's files without the author saying so.
