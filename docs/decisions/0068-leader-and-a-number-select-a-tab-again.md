# 0068: The leader and a number select a tab again, and alt and a number is unbound

**Date:** 2026-11-26
**Status:** Accepted. Amends decision records 0064 and 0065. `leader 1` to `leader 9` select the
tabs again, as they did before MUX-60, and `M-1` to `M-9` have no default binding. `D-1` to
`D-9` still open the agents the Navigator numbers (0065).
**Decision:** `leader 1` to `leader 9` are `tab.select 1` to `9`. Nothing in the defaults binds
`M-1` to `M-9`, so alt and a number reach the pane. `D-1` to `D-9` are the only default keys
that open an agent by its number.

## Context

0064 moved the tabs to `M-1` to `M-9` so that the digits under the leader could open agents. 0065
added cmd and a number for the same agents, and kept the leader's digits on the agents for
terminals and ssh sessions that cannot send cmd. The author now wants alt and a number gone, the
leader and a number back on the tabs, and cmd and a number left as the agent shortcut.

## Consequences

- A reader in a terminal that cannot send cmd and a number, over ssh, or under Omarchy, has no
  number key for an agent. The agents overlay (`leader a`) and the Navigator still reach every
  agent.
- `M-1` to `M-9` reach the pane again. A program that reads alt and a digit, such as readline's
  digit arguments, gets them.
- A config that binds `M-1` to `M-9` keeps its own bindings. The defaults no longer carry them.
- The Keys overlay shows `leader 1-9 tab.select <n>` and `D-1-9 agent.focus #<n>`. It has no
  `M-1-9` row.
