# 0029: Stay awake is a dot, a toast and one key

**Date:** 2026-09-10
**Status:** Accepted. Amends the architecture spec's section 9 table name, its method table,
its schema version note, and answers its open question 1.
**Decision:** The feature is called **stay awake**, in the config table `[stay_awake]`, in the
methods `stay_awake.enable`, `stay_awake.disable` and `stay_awake.toggle`, in the subcommand
`stay-awake`, and in every line the reader sees. A dot at the right end of the top bar says
whether the hold is on: green when it is, grey when it is not. A toast says what changed each
time it changes. `leader A` toggles it. The flag persists, so a hold survives a restart, and
`state.json` goes to schema version 4 to carry it.

## Context

MUX-15 asks for the V1 keep-awake feature in V2, without V1's menu: a command, a shortcut and
a notification when the shortcut is pressed. The architecture spec already scoped the feature
and named it "domux stay alive". Its open question 1 asks where the feature belongs on the
screen and answers nothing. The interface spec draws no surface for it. So the code exists in
plan form and the screen does not.

## Why "stay awake" and not "stay alive"

The author chose the word when the surface was drawn, and one term per concept is a repo rule,
so the plan's word gives way rather than the two of them living side by side. "Stay alive"
also reads as something about the server's own health, which is what a reader of a
multiplexer would guess first, and the feature has nothing to do with it.

The backend program names stay out of the copy for the same reason. V1 called the feature
after the macOS program that implements it, which named the feature after an implementation
detail that does not exist on Linux. One error names a program, and only because the reader
has to install it: the Linux hold needs `systemd-inhibit` and the message that says so is
useless without the word.

## Why a dot rather than a word

The M4 plan proposed the word `awake` at the right end, before the clock. The right end is a
priority chain: it shows the prompt keys, or the chord, or the copy mode keys, or a hint, or
the config error, or the clock, and never two of them. A word placed inside that chain is a
status the screen stops showing the moment anything else has something to say, which is a
status that cannot be trusted.

The dot sits outside the chain, at the far right of whichever surface carries the right end,
and is drawn on every frame. It costs two cells and it is always right. The glyph is the one
the agent rows already use, so the screen has one shape for "a thing that is in a state" and
not two.

Grey when off rather than absent: a dot that appears only when the hold is on has no resting
state, so the reader learns nothing from its absence and cannot find it to check.

## Why a toast and not the hint

The hint at the clock's place is the answer to one key and it stands until the next key, which
makes it the natural home for the answer to `leader A`. It was not chosen, for two reasons.

The toast is where this class of message is going. Interface spec 8 gives the Notifier a toast
for agent transitions, and stay awake's message is the same kind of thing: a state changed
somewhere and the reader should know. Building the surface here means M4's Notifier draws into
a surface that already exists rather than inventing it alongside its own rules.

The hint is also per client and per key. Stay awake is one server-wide state, and a toggle
from the CLI in another terminal has no key and no client to hang a hint on. Every attached
client gets the toast, whichever way the state changed.

The toast carries no configuration yet. The corner is the workpanel's bottom right and the
life is six seconds, both from interface spec 8.1, written as constants. M4 brings
`[notifications.toast]` and reads them from there.

## Why `leader A`

`awake` is the word on screen and `A` is its letter. `a` is taken by the agents overlay, so
the two differ by a shift, which is the same relation `n` and `N` already have for the two
workspace name keys. V1 bound `K` in tmux, after the program name this record just removed
from the copy, so it carries nothing over.

## Why the flag persists

A reader who turns stay awake on has said something about the next few hours, not about this
run of the server. Design principle 11 wants the hold released when domux exits, and it is:
the child process is killed and the process id file is removed. What survives is the flag, so
the next start takes a fresh hold.

That leaves one case the flag cannot answer, which is a server that died without releasing.
`adopt` reads the process id file at start and takes over a holder that is still alive, so the
machine never ends up with two holds or an orphan nothing can kill.

## Consequences

- `state.json` reaches schema version 4 with `stay_awake: bool`, migrated from 3 as `false`.
  The M4 plan reserved 4 for its own fields; it takes 5.
- `CoreDeps` carries a command runner, and the fake one records calls, so no test on any
  machine starts a hold, kills a process or asks for sudo.
- The two files full mode needs on macOS are written by `stay-awake install --full` and by
  nothing else. They are domux2's own pair, at its own paths, so V1's launch daemon and
  sudoers file are left where they are.
- The architecture spec's section 9 `[stay_alive]` table, its `stay_alive` method row and its
  "4 at M4" schema note no longer describe the build. This record replaces all three.
