# 0054: A program claims the focus keys

**Date:** 2026-09-14
**Status:** Accepted. Changes the default `[keys.passthrough] commands` that interface spec 4.4
set.
**Decision:** A program in a pane can claim the passthrough keys with `pane.claim_passthrough`.
domux then passes those keys to the pane while the program is in the pane's foreground process
group, and the program hands focus back with `focus.<dir>` and its own pane. The Neovim plugin in
this repository does both. `nvim` and `vim` leave the default passthrough commands, so without
the plugin the keys move between panes.

## Context

MUX-45. In the author's LazyVim session in the right pane, `C-h` did not reach the Claude pane on
the left. domux passed `C-h`, `C-j`, `C-k`, `C-l` and `C-\` to any program named `nvim`, `vim` or
`fzf`. The author's Neovim mapped the keys to vim-tmux-navigator, which asks tmux to move when
Neovim has no window in that direction. Under domux `$TMUX` is empty, so the plugin ran
`wincmd h`, which did nothing at the left edge, and nothing told domux to move.

Only Neovim knows whether it has a window in the direction pressed, so part of the answer has to
run in Neovim. This record answers how domux knows which programs will hand the key back.

## The claim

- The caller claims, and nothing else can. `pane.claim_passthrough` takes the process at the
  other end of the socket, which `socket::control` reads from the peer credentials (decision
  0045). No parameter names a process.
- A claim holds while its process is in the pane's foreground process group, the group
  `tcgetpgrp` answers for the pane's PTY. That one rule answers each case:
  - `C-z` in Neovim, or Neovim ending, puts the shell in front, so the next key is domux's.
    Neovim does not have to release anything.
  - `git commit` runs Neovim in git's group, so the claim holds although the pane's command is
    `git`, which a list of names cannot express.
  - A process outside the pane is never in its foreground group, so its claim never holds, and
    nothing needs checking at claim time.
- A pane holds a set of claims. Neovim's `:terminal` can run a second Neovim with the pane's
  variables, and its claim must not replace the first. It never holds either: it runs on
  Neovim's own PTY.
- The check runs at the key press, not in the once-a-second observation, and only for a
  passthrough key on a pane that has claims.
- A key pressed while a box has the keys is the box's, so no passthrough rule applies then.
  Before this record, a passthrough command in the pane beside the sidebar kept `C-l` from
  leaving it.
- Claims cross an upgrade in the handover, as a list of process ids on each pane. An older build
  ignores the field and a handover without it reads as no claims, so `HANDOFF_FORMAT` stays 1.

## Handing focus back

`focus.left`, `focus.right`, `focus.up`, `focus.down` and `focus.last` take an optional `pane`.
With it the move happens only while that pane has the keys. A second `C-h` that Neovim sent before
the first one landed finds the keys elsewhere and changes nothing, rather than moving the reader
again.

## The default

`[keys.passthrough] commands` is `["fzf"]`. With `nvim` in the list, a Neovim without the plugin
kept the keys and never gave them back, which is this issue. A user who has not added the plugin
now moves between panes, and one who wants the old behaviour adds the names back.

## Alternatives

- **Keep the names and ship only the plugin.** A Neovim without the plugin keeps the keys, and a
  Neovim that `git commit` starts gets none.
- **An installer that writes Neovim's configuration.** It cannot know how a configuration is laid
  out, and LazyVim sets its own maps after a plain plugin file has set its own.
- **Ask Neovim over its RPC socket.** domux would rebuild Neovim's modes and maps from outside it,
  and every key would wait on a round trip.
- **Set `$TMUX` so vim-tmux-navigator works unchanged.** Every program that checks for tmux would
  then believe it runs in tmux, image.nvim among them.

## Consequences

- The plugin lives at the root of this repository, in `plugin/` and `lua/domux/`, so it changes in
  the same pull request as the API it calls, and a lazy.nvim spec pins it to a release.
- A claim made through `domux api` belongs to the CLI process, which ends at once, so there is no
  subcommand for it.
- The plugin connects once per request. A connection held open would hold up an upgrade's drain
  for its ten seconds.
- `tests/nvim/requests.jsonl` holds every request the plugin sends. The plugin's suite checks that
  it sends only those, and `domux-core` parses each one, so the suite's fake server cannot drift
  from the real one.
