# 0017: Focus with nothing attached seats the next client

**Date:** 2026-09-10
**Status:** Accepted
**Decision:** `workspace.focus` made when no client is attached records the workspace as
`last_workspace` and answers with it, rather than refusing. The client that attaches next
seats there, because `last_workspace` is the field `Core::attach` already reads. A call that
names a client the model does not hold still refuses and still changes nothing.

## Context

MUX-11 reported the bare command failing in a directory that was not a project:

    ❯ domux2 project remove dotfiles --yes
    ❯ domux2
    /Users/pranav/dotfiles is not a project yet. Register it? [y/N] y
    not_found: no client is attached; run domux2 to attach one

The offer of decision record 0009 runs the two calls `open` runs, `project.add` and
`workspace.focus`, and it runs them before the attach so the question lands on the plain
terminal. That ordering is the point of 0009 and it is right. What it means is that the focus
is made at the one moment when there is nothing attached, so `Ctx::view` had nobody to name
and refused, and the `?` on that call took the attach down with it. The reader was left with a
registered project, no screen, and a sentence telling them to run the command they had just
run.

`open` typed in a terminal with nothing attached failed the same way and had a test pinning
it. That test is now the other way round.

## Why the field and not a new one

Nothing new is remembered. `Model::last_workspace` already means "where the next client
sits": `Core::attach` seats a client on it, falling back to the first workspace, and the
state file has carried it since schema version 1. A switch with no view is exactly a write to
that field with no view to move afterwards, so the handler stops one statement earlier rather
than doing something different.

## Why a named client still refuses

Naming `c_9999` and naming nobody are different requests. The first asks for that client's
screen, and there is no honest way to give it one; answering ok would report a switch that
did not happen. The guard that refuses it was written for the same reason this record exists,
that a call must change nothing rather than half of it, and it is unchanged. Only the case
where the caller named no client and none is attached is answered.

## Consequences

- `Ctx::view_or_none` is the new half of `Ctx::view`, and `view` is written in terms of it.
  `workspace.focus` is the only caller; every other method asks `view` and refuses, because
  every other method moves something on a screen.
- No `workspace.switched` event is published on this path, and no view is marked dirty. The
  event names the client it switched and there is none.
- The attach offer and `open` both work with nothing attached. Neither command changed.
- A refusal that arrived after `project.add` had run was the shape of this bug: the
  registration was true and the sentence said the opposite. The commands still say the
  adoption first, for the reason recorded on them.
