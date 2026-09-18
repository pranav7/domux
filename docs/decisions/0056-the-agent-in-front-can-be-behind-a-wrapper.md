# 0056: The agent in front can be behind a wrapper

**Date:** 2026-09-18
**Status:** Accepted. Widens what the observer reads, which architecture spec 3.5 and decision
record 0030 leave as "the foreground process".
**Decision:** The agent in front of a pane is the process in front when a manifest names it, and
otherwise the first agent the manifests name in that process's group, nearest to it first. The
record holds the agent's own process id, not the wrapper's. `agents::front::agent_in_front`
answers it, from `ProcessInspector::group_members`, and the observer is its only caller.

## Context

MUX-54, the other half. The author's Codex sessions had no row in the Navigator at all, even
when a session was started by hand in a pane.

Two sources write records, and neither one wrote this one:

- The hooks reported a pane that was not the session's. Recent Codex builds keep sessions on a
  shared local app-server daemon, and a daemon that another pane started carries that pane's
  `DOMUX_PANE`. Decision record 0055 stops such a report taking the pane it names, which is
  right, and leaves the session with no row.
- The observer never saw the agent. It read the name of the leader of the pane's foreground
  process group, and the author starts Codex through `~/.local/bin/codex`, a script. A pane
  running a script is led by `/bin/bash`, so the answer was "no agent is running here".

Reproduced on Linux against a server of this build, with a script in a pane and a process named
`codex` below it:

```
bash ← zsh          the wrapper in front, codex in its group: no record at all
codex ← zsh         the same agent, started directly:         a record within a second
```

Every wrapper does this, not only a script of the author's own: a version manager's shim, and an
npm launcher that runs the agent from `node`. The agent is always in the group, because the
wrapper runs it in the pane and the pane's foreground group is what the wrapper leads.

## The rule

The question the observer asks is "what agent is running in front of this pane", and the process
in front is only the first place to look. The rest of its group is the second, nearest first.

`ProcessInspector::group_members` reads the group: macOS lists one with `proc_listpgrppids`,
and Linux, which lists none, walks what the leader started and keeps what stayed in its group.
A descendant that left the group is not in the pane, which is exactly what a session on a
daemon of its own has done.

The process the record holds is the agent's, not the wrapper's. It is what ends the record when
it goes, and what a hook's walk up passes through (`nested::runs_under`, decision record 0055),
so binding the wrapper would answer for a process that is not the agent.

The agent nearest the leader is the one, because an agent that agent started is a nested agent
and a nested agent is not the pane's agent (decision record 0045). Nothing else about the
observer changes: a record still ends only when its own process goes or its pane goes, never
because the foreground changed.

## What this leaves

- **A row with no session name and no state.** The observer's record is `unknown`, and a Codex
  session whose hooks name another pane never names it. That is what a session domux cannot
  hear from looks like, and it is the promise of 0030: a record exists whether or not the hooks
  are installed.
- **A wrapper that starts its agent with a session of its own** puts it outside the group, and
  the pane has no agent to show. `nohup` and `setsid` do this; no wrapper in normal use does.
- **The pane's border still says the wrapper**, because that is the command the pane is running.
  Only the agent question reads past it.
- **A group is read to 64 processes.** A pane whose group is larger than that is a pipeline, not
  an agent behind a wrapper.

## Alternatives

- **Read the Linux `comm` instead of `argv[0]`.** The kernel puts a script's own name there, so
  `~/.local/bin/codex` would read `codex`. It answers one wrapper on one platform: the author's
  script hands over to a version manager, which leaves `mise` in front, and macOS names the
  interpreter. Decision record 0026's reason for `argv[0]` also stands: a versioned install
  resolves to a file named after its version.
- **Match the agent by the command line rather than the process name.** A manifest would carry
  patterns, and every pane running `vim ~/notes/codex.md` would hold an agent record.
- **Let the hooks say which pane they are in, from the caller's tree.** The caller is the
  daemon, and a daemon is in no pane.

## Consequences

One question per pane per tick, answered from the process table the inspector already reads.
On Linux that is one read of the group leader's children, and one of each child's stat, only
for panes whose foreground process is not itself an agent.
