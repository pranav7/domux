# 0065: Cmd and a number opens an agent, and the numbers show while cmd is held

**Date:** 2026-11-26
**Status:** Accepted. Amends decision record 0064: the agents gain `D-1` to `D-9`, and the
Navigator writes their numbers only while the super key is held, at a client that can tell. The
rest of 0064 stands. **Amended by decision record 0068**, which gives `leader 1` to `leader 9` back to the tabs and
unbinds `M-1` to `M-9`, so the agents keep `D-1` to `D-9` only. **Amended by decision record 0066**,
which adds `domux install ghostty` and
reads cmd only where Ghostty confirms that cmd and a number reaches domux.
**Decision:** `D-1` to `D-9`, cmd and a number on macOS, open the agent the Navigator numbers
so. `leader 1` to `leader 9` still do, and `M-1` to `M-9` still select the tabs. A client on the
reader's own Mac asks macOS whether cmd is held while its terminal has the focus and tells the
server, and the Navigator writes the agents' numbers only while it is. A client that cannot tell
says nothing, and the Navigator writes the numbers for it all the time.

## Context

After MUX-60 the author read `leader 1` for an agent and `M-1` for a tab as one shortcut that
did two things. They asked for the agents on cmd and a number and the tabs on alt, and for the
numbers next to the agents to show only while cmd is pressed.

## The keys

`D-` is the super modifier in a key name, which is cmd on macOS. `D-1` to `D-9` are global
bindings to `agent.focus #1` to `#9`, beside `M-1` to `M-9` for the tabs, so the modifier says
which list a number counts in. The Keys overlay collapses them into one row, `D-1-9
agent.focus #<n>`.

The terminal has to let the chord through. Ghostty binds cmd and a number to its own tabs twice,
by the character and by the key, so its config needs `keybind = cmd+1=unbind` through
`cmd+9=unbind` and `keybind = cmd+digit_1=unbind` through `cmd+digit_9=unbind`. `domux install
ghostty --apply` writes them (decision record 0066). Ghostty then sends the chord to the program, and under the keyboard protocol the client already asks for, it arrives as super and
the digit. Hyprland, under Omarchy, binds super and a number to its workspaces and never sends it
on. So `leader 1` to `leader 9` stay bound to the agents: they reach domux from any terminal and
over ssh.

## Seeing cmd held

A terminal program is sent keys, not modifiers on their own. The kitty keyboard protocol reports
cmd pressed and released only with two more flags: report every key as an escape code, and report
releases. In that mode a key that composes text, such as `e` after a dead key, or a letter with
Option where Option is not alt, arrives as its base key, with the text in a field of its own.
crossterm 0.29, which reads the client's input, does not read that field, so `é` would reach
every pane as `e`. Keeping the text would mean the client parsing every key, mouse and paste
sequence itself instead of crossterm.

So the client asks the operating system. On macOS, `CGEventSourceFlagsState` answers which
modifiers are held in the login session, and needs no permission, where reading the other keys
would. The client asks every 50 ms while its terminal has the focus, which it knows from the
terminal's focus reports, and sends `ClientMsg::SuperHeld` once at attach and then whenever the
answer changes. When the terminal loses the focus it sends `false`, because a key held in another
window is not held here, and stops asking until the focus comes back. A client over ssh asks
nothing, because the machine it runs on is not the one the reader is at. `DesktopEnv::remote`
already says which that is.

`super_key::POLL` is 50 ms so the numbers come with the key, which is the moment the reader
looks for them.

## Drawing

`ClientView::super_held` is `None` until the client says. `ClientView::shows_agent_numbers` is
true while it is `Some(true)` and while it is `None`, so a client that cannot tell, over ssh, on
Linux, or on another system, sees the numbers all the time, as 0064 drew them. Each client draws
its own: cmd held at one screen does not show the numbers at another.

The row keeps the number's cells while the number is hidden. A name the number shortened stays
that short, and nothing in the row moves when cmd goes down.

The numbers come with any cmd chord, cmd and c as well, for as long as cmd is down. A delay
before they show would hide them from the reader who presses cmd to read a number and then the
digit, which is the reader they are for.

## Protocol

`ClientMsg::SuperHeld` is a new message, so `PROTOCOL_VERSION` is 5. Like `Colors`, it is not
the reader's activity: it moves no most recent client and touches no pane.

## Consequences

- In Ghostty, cmd and a number reaches domux only once the config unbinds it, and Ghostty's own
  tabs then lose those keys. `ctrl+cmd+[` and `ctrl+cmd+]` still move between them.
- Under Omarchy, super and a number stays Hyprland's. The numbers show all the time there,
  because that client cannot tell when super is held.
- `D-1` to `D-9` no longer reach a program in a pane. Few terminal programs read super, and a
  config that needs them unbinds the keys with an empty action.
- A client on a Mac asks macOS 20 times a second while its terminal has the focus, and not at all
  while it does not.
