# 0033: The agents overlay outlives the Navigator

**Date:** 2026-09-11
**Status:** Accepted. Amends decision 0030.
**Decision:** `leader a` opens the agents overlay whatever `[navigator] enabled` says. The
overlay lists the agents alone, under a header per project, the one that most wants you first.
Only the sidebar's Agents box goes when the key goes.

## Context

Decision 0030 combined the two boxes into the Navigator and made `leader a` do nothing with the
Navigator on. The reasoning was that the reader who pressed it finds the same records one key
away, so a key that only refuses is a key with nothing to say.

The author has lived with the one list and wants the agents on their own back, to find out over
a few weeks which one they reach for. That is what this record is: the key comes back, and both
views stand until there is evidence for dropping one.

## The two views answer different questions

The Navigator answers where an agent is. It nests the agent under its workspace, under its
project, and it never reorders, so nothing moves under the reader while a state changes. Finding
a particular agent means knowing where it runs, which is how the author navigates.

The agents overlay answers which agent wants you. It drops every workspace that holds no agent,
so a machine with ten projects draws only the rows that are doing something, and it keeps the
attention order `Model::sorted_agents` already computes: a waiting agent comes first, and its
project header rises with it. That order is the thing the Navigator deliberately gives up, and
this is the surface that can keep it.

Both are true at once and neither is a layout of the other, so the two live side by side rather
than one being a mode of the other.

## What does not change

The overlay is the one MUX-21 built and nothing about it moves. The rows are
`agents_box::rows` in `RowForm::Overlay`, which is the same grammar the Navigator draws through
`one_row`, so an agent reads the same way wherever it is met. `agents.close` still gives back
the overlay underneath. The record lifetime, the dot rule and the Navigator's own order are the
whole build's and are untouched here.

No new config key. `leader a` is a binding in `[keys.bindings]` like every other, and a reader
who does not want the overlay unbinds it.

## What this amends in 0030

Decision 0030 said that on, `leader a` does nothing at all, and that the agents overlay retires
with `[navigator]`. Both halves go. `leader a` opens the overlay in either layout, and the
deletion the key still names is the sidebar's two boxes: the Projects box, the sidebar's Agents
box and `RowForm::Sidebar`. `RowForm::Overlay` survives it.

The Agents box is a live word again, and it names the box inside the agents overlay. The
sidebar's Agents box keeps the same name and is the half that goes.

## This is an experiment, and it is written down as one

The author is comparing two surfaces, not adding a permanent second one. If the overlay earns
nothing over the Navigator, it goes, and the case for that will be the same kind of evidence
0030 asks for about `unseen`: which surface was actually opened, and what the other one failed
to answer. Until then the cost is one key and one row form, both of which are already built.
