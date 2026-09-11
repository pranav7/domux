# 0035: Wide agent rows name the pane

**Date:** 2026-09-11
**Status:** Accepted. Amends decisions 0030 and 0033.
**Decision:** The agents overlay and the switcher name the pane after the tab. The sidebar and
the Navigator stay unchanged.

## Context

An agent runs in a pane, but its wide row stopped at the tab. Two agents in two panes of one
tab therefore drew the same `workspace › tab` place. More importantly, neither row said the
last part of where the work was running.

The pane box already names the pane with its foreground command. The agent row now uses the
same name. A row under the `DOMUX` project header can read `audrey-app › dot cleanup › node`:
workspace, tab, pane.

## Wide rows have the room

The agents overlay adds the pane on line two. The switcher adds it after the tab on the first
line. Both use `›`, because the pane is the next level inside the tab.

The sidebar and the Navigator keep their compact rows. The pane name is absent until the
process inspector has reported one, so the row never guesses. Enter still focuses the pane in
the agent record.

The pane name is part of the filter text, so `/ node` finds these rows.

## Consequences

- A wide row gains one field whenever the pane box has a name.
- The pane box and the agent row change together when the foreground command changes.
- Two panes can have the same name. The row reports the name instead of inventing another one.
