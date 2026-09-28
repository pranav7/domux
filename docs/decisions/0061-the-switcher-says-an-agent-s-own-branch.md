# 0061: The switcher says an agent's own branch

**Date:** 2026-09-28
**Status:** Accepted. MUX-55. Changes the switcher's agent row from 0030 and 0038, and adds an
agent scope to the facts.
**Decision:** The switcher's agent row reads like the sidebar's: the name in its kind's colour, then
the glyph with no working word. Where the kind, the tab and the pane were, it says the branch and
pull request of the directory the agent works in, and only when that branch is not its
workspace's. The branch and pull request providers look at every agent's directory as well as at
every workspace.

## Context

An agent can work in a worktree it made for itself rather than in a domux workspace. Claude Code
does this under `.claude/worktrees/`, inside the workspace's own directory, so the agent sits under
a workspace whose row says one branch while the agent commits to another and opens its own pull
request. The switcher had no way to say so.

The author also found the switcher's agent row hard to read. It said the kind, the tab and the
pane after the activity (`claude › t_b74a › claude`), which is where the agent runs and not what it
is doing, and it said the working word, which the sidebar had already dropped in 0038 because it
adds clutter and nothing a reader acts on.

## The row

`└ audit-planning ✶  feature/eng-533-session-polish · PR#1287`, then the recap as before.

- **The name is in its kind's colour**, as the sidebar's nested row draws it since MUX-42. The kind
  was the first word of the tail and is gone, so the colour is what says which kind is running.
- **The glyph stands alone.** A working row turns the star and a compacting row breathes the arrow,
  as in the sidebar. The agents overlay, `leader a`, keeps the word: it is the one surface that
  still has it, and it is the surface for which agent wants you (0033).
- **The tail is the agent's branch and pull request**, in the workspace row's colours and order: the
  branch in `branch`, the number in its state's colour. The branch gives way first and the number
  never does. The name keeps its first sixteen cells before the branch gives way to it, and a tail
  with no room for the number is not drawn at all.
- **An agent on its workspace's branch adds nothing.** The workspace row directly above already
  says that branch and that pull request, and saying them twice is the repetition MUX-21 removed.
  So the tail is there exactly when it tells the reader something the rows above do not.
- `/` also matches the agent's own branch and pull request number.
- The sidebar's nested row and the agents overlay are unchanged.

## The facts

`FactScope::Agent` is a fourth scope, keyed `a_5e21/branch` and `a_5e21/pr`.

- A provider opts in with `FactProvider::follows_agents`. The branch and the pull request do; an
  extension's provider does not, and gets its workspaces as before.
- An agent's target is the directory its hooks last reported as `cwd`, which is `Agent::cwd`. It
  takes its workspace's project root, default branch and handle, so the pull request provider
  refuses the same recycled branches for an agent that it refuses for a workspace.
- The target carries the workspace's branch too, and the pull request provider does not ask `gh`
  about an agent on that branch. Without it every agent would cost a second `gh pr list` a minute
  for an answer the switcher does not draw.
- An agent's pull request is cached with the rest, and its facts go when its record does, through
  the same `forget_deleted` sweep a workspace's go through.

## Considered

- **Showing the branch on every agent row.** Rejected: under a workspace every agent on its branch
  would repeat the line above, and the one agent in a worktree of its own would not stand out.
- **Asking git for the branch from the pane's directory.** Rejected: a pane's directory is the
  shell's, and an agent that moved into a worktree reports the new one in its hooks while the
  shell stays where it was.
