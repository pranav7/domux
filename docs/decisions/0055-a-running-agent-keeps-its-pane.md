# 0055: A running agent keeps its pane

**Date:** 2026-09-17
**Status:** Accepted. Widens decision record 0045, which asked the process tree alone whether a
hook came from a nested agent. The tree is still asked; the pane's own record is asked as well.
**Decision:** A hook never takes a pane from a record whose own process is still running, unless
the hook ran under that process. `agent.report` walks up from the process at the other end of the
socket, as 0045 has it, and looks for the record's process id rather than counting agents by
name. A hook that would displace such a record and did not run under it is answered the way a
session domux is not tracking is answered: no record changes, and there is no context block.

## Context

MUX-54. The author's Claude session started a Codex worker, and the Codex row replaced the
Claude row in the Navigator. That is the MUX-43 symptom, which 0045 was written for, from a
tree 0045's walk cannot read.

`is_nested` counts the processes a manifest names, from the hook up to the server, and says
"nested" at the second one. It needs the worker's tree to still pass the agent that started it.
Reproduced on Linux against a server of this build, with a fake `claude` in front of a pane and a
`codex` started from it:

```
codex ← claude ← systemd            the worker, still a child of its agent: dropped
codex ← systemd                     the same worker, reparented: takes the row
```

A worker loses that tree in more than one way. The shell that ran it exits first, which 0045
records as a case it does not cover. Or the session runs somewhere else entirely: recent Codex
builds keep sessions on a shared local app-server daemon, and a daemon is detached by design.
Either way the hook counts one agent above it and no second one, `Model::report_agent` reads the
new session id on a pane that holds another as a session taking the pane, and the Claude record
ends.

What follows is worse than a wrong row. The observer makes no second record on a pane that
already holds one, so the Claude row does not come back while that session runs, and the worker's
`SessionEnd` takes the row away for good. Confirmed in the reproduction: the pane's Claude was
alive and in front of its pane for as long as the row was gone.

## The rule

The evidence 0045 does not use is the record itself. A record with a process id that is still
alive is a live agent in that pane, and there is exactly one row per pane. So the question moves
from "how many agents are above this hook" to "did this hook come from the agent whose row it
would take".

`nested::runs_under` answers it from the same walk, and takes no names: the pane's own agent
answers for a hook of its own whatever is above it, and a worker it started does not.
`api::agent::displaced_agent` says when to ask, which is when the report would take the pane from
another session. Two reports take nothing:

- one whose session id the pane's record already holds, which is the pane's agent reporting
  again;
- one whose kind matches a record the observer made and no hook has named, which is the pane's
  agent reporting for the first time.

A session id that changes under a process that did not, which is what Claude's `/clear` does,
is neither of those: it takes the pane, and its hook ran under the record's process, so the walk
lets it through. This is the case 0045 named when it rejected "ignore a hook with another session
id while the record's process is alive"; the walk is what tells the two apart, and the walk is
still here.

## What this leaves

- **A record with no process id is not protected.** Only the observer binds one, and it binds
  only what is in front of a pane, so an agent running inside an editor's terminal has no process
  id on its record. Its hooks are its only source and they still land, which is 0045's behaviour.
- **A hook that arrives while the pane's agent is dying** is read the way the inspector reads it.
  A process the OS still answers for holds the row for another tick.
- **A worker of the pane's agent's own kind, on a record with no process id**, is still only
  caught by the count in `is_nested`.
- **Nothing here gives the row back.** A row a report took before this build stays taken until
  that session ends: no hook is made good later.

## Alternatives

- **Let only the same kind take a pane.** It answers MUX-54 and not MUX-43, where both sessions
  are Claude, and it makes a real handover from one kind to another wait for the observer's tick.
- **Bind a process id from the hook's own walk**, so a record the observer never saw is protected
  too. The nearest agent above a hook is the session that ran it, which is the worker in the very
  case this record is about, so the first hook to arrive would bind the wrong process.
- **Ask the pane's foreground process instead of the record.** 0045 rejected it and the reason
  holds: an agent inside an editor's terminal is never in front, and its hooks would be dropped.

## Consequences

One more walk of a few processes, on reports that would displace a record and only those. The
walk `is_nested` already does is now read to the end rather than stopped at the second agent,
which is what it did in the negative case before.
