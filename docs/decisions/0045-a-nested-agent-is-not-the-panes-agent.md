# 0045: A nested agent is not the pane's agent

**Date:** 2026-09-14
**Status:** Accepted. Narrows the rule in `Model::report_agent` that a second session on a pane
ends the first record: the rule holds for the pane's own agent, and a nested agent's hooks never
reach it.
**Decision:** A nested agent is an agent that another agent started, such as a worker a session
hands a task to from its shell. `agent.report` reads the process id at the other end of the
socket and walks up from it. When two processes on the way to the server have names a manifest
lists, the hook came from a nested agent, and `agent.report` answers it the way it answers a
session domux is not tracking: no record changes, and there is no context block.

## Context

MUX-43. The author's session in the `agent-harness` workspace handed a rename to a worker with
`sub-claude`, a script that runs `claude -p`. The session ran it as a background Bash tool call.
The worker inherits the pane's environment, `DOMUX_PANE` included, so every hook it ran reported
from the session's pane.

`Model::report_agent` reads a hook whose session id no record holds, on a pane whose record holds
another, as a new session taking the pane. It ended the session's record and made one for the
worker. The session's next hook found the worker's record on the pane and did the same in
reverse. So the two sessions traded the row on every hook:

- The row lost its session name, because a new record has none until the tick reads the
  transcript.
- The row moved to the end of its workspace, because agents sit in the order they started.
- The row showed the worker's state half of the time.
- When the worker ended, its `SessionEnd` took the row away, and the session came back as an
  `unknown` record the observer made.

The author's report is that a worker must not change the session's row, and must not have a row
of its own.

## The rule

A hook runs under the agent that ran it, and that agent runs under the pane's shell. A nested
agent's hook has a second agent above the first. This is the tree the worker's hook ran in,
captured with `ps` from a hook of a `claude -p` started from this session's Bash tool:

```
sh              the hook command
claude -p       the worker
zsh -c          the Bash tool call
claude          the pane's agent
-zsh            the pane's shell
domux server    the server
```

`agents::nested::is_nested` counts the processes whose name `Registry::for_process` recognizes,
starting at the caller and stopping below the server, and answers yes at the second one. It reads
names through `ProcessInspector::name_of`, which is the argv[0] reading `foreground` already uses,
so the walk recognizes an agent by the name the observer does. Any kind counts: a Claude started
by Codex is as nested as a Claude started by Claude.

The walk stops below the server because nothing above the server is in a pane. A server started
from inside an agent's shell would otherwise mark every agent in it as nested. The walk also stops
at a process it has already read, because it reads the table one process at a time while
processes come and go, and a loop would count one agent twice.

The walk only drops a hook on evidence. A walk that ends early, at a process the OS no longer
answers for or one whose parent it can't read, finds at most one agent and lets the hook through.
That is the behavior before this record.

## Why the socket and not a parameter

The hook could send its own process id in `AgentReportParams`. But `AgentReportParams` refuses
unknown fields, and a hook runs the binary that installed it (decision record 0040). After an
upgrade, the server that answers is the older binary until it restarts, so every hook would fail
against it and every row would stop updating. The CLI swallows the error, so nothing would say
why.

The socket's peer credentials need nothing from the hook. `socket::control` reads them once per
connection, `CoreMsg::Api` carries the id, and `Ctx::caller` holds it for the handler. A key press
has no caller. A hook from an older binary gets the check as soon as the server has it.

## Alternatives

- **Ignore a hook with another session id while the record's process is alive.** Claude changes
  session id without changing process on `/clear`, so this rule needs to know which process sent
  the hook, which is the walk. The `source` field of `SessionStart` doesn't separate the two
  cases, because a worker can be started with `--resume`.
- **Count only hooks whose agent is the pane's foreground process.** A worker in a background
  shell isn't in front, but neither is an agent run inside an editor's terminal. The observer
  makes no record for that agent, so its hooks are its only source, and this rule would drop them.
- **Mark the agent's shell with a variable.** Claude lets a `SessionStart` hook export variables
  into its Bash tool through `CLAUDE_ENV_FILE`, so a worker could inherit a marker. Codex and
  OpenCode have no such file.
- **Show the worker as a row under the session.** The report asks for the session's row alone.

## What this does not do

- A worker whose parent shell exits before it does, such as `nohup claude -p ... &` in a call that
  returns at once, is reparented to launchd or init. Its tree no longer passes the agent that
  started it, so its hooks still take the pane. A background Bash tool call keeps its shell until
  the command ends, which is how the orchestrator skill starts workers.
- An agent whose process name no manifest lists isn't counted. The observer has the same limit.
- `domux whoami` run by a worker answers with the pane's agent, because it asks by pane.

## Consequences

A nested agent gets no context block, so nothing tells it that it runs in domux. Every hook costs
a walk of a few processes on the core task, one `proc_pidinfo` call or one read of
`/proc/<pid>/stat` per process, plus the name read `foreground` already does.
