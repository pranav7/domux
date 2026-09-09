# 0009: Attach offers to register the directory it was typed in

**Date:** 2026-09-09
**Status:** Accepted. Extends decision record 0006, which stands.
**Decision:** Before it attaches, `domux2` and `domux2 attach` ask once whether the directory
they were typed in should become a project, when that directory is not one already and is not
under one. Answering yes runs the two calls `open` runs, `project.add` and `workspace.focus`.
Answering anything else, end of input included, registers nothing and attaches as before.

## Context

Decision record 0006 settled that attach reconnects to what is registered rather than
following the shell, the way `tmux attach` reconnects to a session's existing windows. That
rule is unchanged and this does not weaken it: nothing here moves a client to a directory on
its own.

What 0006 left, once M2 shipped projects, was a gap rather than a rule. Typing the bare
command in a new directory landed you in another project's tabs with nothing on the screen
about the directory you were standing in, and the way out was to already know that `open`
exists. MUX-6 reported that as "adding a project": attach in a directory should be able to add
one.

## Why it asks rather than registers

A bare command typed in a downloads folder, a scratch directory or somebody else's checkout
would otherwise leave a project behind that nobody asked for, and removing one is a
destructive call that asks first. The question costs one keystroke and its default is the
answer that changes nothing.

## Consequences

- The question is asked on the plain terminal, before raw mode and before the client draws,
  so it is not a second thing on a screen that already has an accent border (principle 2).
- It is skipped whenever there is nobody to answer: standard input or standard error not a
  terminal, a working directory that cannot be read, a `project.list` the server will not
  answer. The attach is what was asked for; the offer is an extra and never blocks it.
- A server this command had to start needs no offer: `Core::new` seeds the directory it was
  started in, so the directory is registered before the question could be asked.
- Reattaching from a directory that is not registered asks every time. There is no record of a
  declined directory and there should not be one: a file of "do not ask about this path" is
  state nobody can see and nobody would think to clear. If the asking becomes a nuisance, the
  answer is to register the directory or to type the bare command somewhere that is.
- `project remove --all` leaves a server with no project, and this offer is how a client gets
  back from there. The refusal a client meets in the meantime names that way out rather than
  only stating the state.
