# 0016: A pane does not inherit the server's environment

**Date:** 2026-09-09
**Status:** Accepted
**Decision:** A pane's environment is built from a keep list, not inherited. `pane::KEPT` names
what a pane takes from the server process - identity and paths, locale and time zone, the ssh
agent and the display, the XDG directories, the macOS login session - plus every `LC_*`.
Everything else in the server's environment is dropped. On top of that go `TERM`, `COLORTERM`
and the pane's own names (`SpawnRequest::env`: `DOMUX_*` and `SHELL`).

## Context

`c`, an alias for `claude`, opened a Bedrock session inside a domux pane while the same alias
opened a subscription session in a terminal and in tmux. The alias was not involved.

The running server (pid 42026) held `CLAUDE_CODE_USE_BEDROCK=1`,
`CLAUDE_CONFIG_DIR=~/.claude-bedrock`, `AWS_PROFILE`, `AWS_REGION`, the `ANTHROPIC_DEFAULT_*`
model pins, and the session markers `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`,
`CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_PID` and `CLAUDE_CODE_MESSAGING_*`. That set is one shell's
session state: an agent working on this repository redeployed the binary with
`domux2 server restart`, run from its own Bedrock session, and `server start` passes its
environment to the daemon untouched. `CommandBuilder::new_default_prog` then starts every pane
from `std::env::vars_os()` of the server, so each pane's login shell was handed that state.

It is self-sustaining. A pane that inherits the markers runs an agent that is itself a child
session on Bedrock, and the next restart that agent performs injects the same variables again.
The marker even hides its own cause: a child session saves no transcript, so the restart at
18:01:33 left no record of the command that made it.

Removing names one at a time - which is what the earlier `TMUX`, `TMUX_PANE`, `TERM_PROGRAM`,
`TERM_PROGRAM_VERSION` removal did - only ever catches the variables already found. The list
that matters is the short one: what a pane genuinely needs. A pane runs a login shell, so
everything a shell config exports is set again inside the pane, and the keep list only has to
carry what a login shell cannot work out for itself.

## Consequences

- A pane is now the same shell wherever the server was started from, and stays that way for the
  daemon's whole life. Whatever a shell config exports still arrives, through the login shell.
- A variable that is exported nowhere but the invoking shell no longer reaches a pane. That is
  the point of the decision, and it is the one way it can surprise: `FOO=1 domux2` puts nothing
  in a pane. Config, not the environment, is where a pane's settings belong.
- `SECURITYSESSIONID` is kept deliberately. It is how a process reaches the macOS keychain,
  which is where the credentials of the programs a pane runs are kept, so a pane without it
  could be a shell that cannot log in to anything.
- A pane started with an explicit command gets the same list. It runs no shell config at all,
  which is why `PATH`, `HOME` and `SHELL` are on the list rather than left to a login profile.
- The server's own environment is still whatever started it. Nothing else it spawns - `git`,
  `gh` - is covered by this decision.
- Running panes are unaffected until the server restarts, because a process keeps the
  environment it was given.
