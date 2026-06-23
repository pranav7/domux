---
name: domux-communicate
description: Use when you (an agent in one domux worktree) need to hand work to, message, or read the output of the Claude agent running in ANOTHER worktree. Covers the canonical peer handoff — "I built X on branch Y, please pull it in" — plus discovering which agents are running. Wraps `domux peek` / `domux send` / `domux read`.
---

# /domux-communicate — message a peer agent in another worktree

domux runs each worktree's Claude agent in its own tmux pane. These three
commands let you address a **peer agent by worktree name** — you never scan
panes yourself; domux resolves the name to the right pane from the session
mapping it already maintains.

The canonical use is a **handoff**: you finished something on your branch and
the agent in another worktree needs it ("I built X on branch Y, here's the doc,
please pull it in").

> The peer is another Claude agent, not the human operator. Every message you
> send is automatically prefixed to say so, so the peer doesn't mistake it for
> its user.

## 1. Discover who's running — `domux peek`

```
domux peek
```

Lists every running Claude agent across worktrees: its **NAME** (what you pass
to `send`/`read`), **STATE** (working / waiting / idle), **TASK**, and tmux
**TARGET**. Run this first if you're unsure of the exact name.

## 2. Send a message — `domux send <name> <message…>`

```
domux send workspace-2 "I pushed the fix on branch eng-225-agent-calc. Pull it: git fetch && git checkout eng-225-agent-calc. The doc is docs/eng-225.md."
```

- **Name** is the worktree's session name, its directory/branch basename, or its
  domux label — any of them resolve.
- **Flags come before the positionals** (Go flag parsing stops at the first
  non-flag word):
  - `--from NAME` — how to attribute the message. Defaults to your own session's
    label or name, so the peer sees who it's from.
  - `--pane W.P` — pick a specific pane (e.g. `--pane 2.0`) when a session runs
    more than one agent.
  - `--no-enter` — type the message into the peer's input box but **don't**
    submit it, so a human (or you, later) can review before sending.
  - `--wait` (with optional `--wait-timeout 2m`) — block until the peer is idle,
    then send. Without it, the message sends immediately; if the peer is
    mid-generation Claude Code **queues** it (nothing is lost) and `send` tells
    you it was queued.

The whole message is sent literally via tmux, so backticks, quotes, `$`, paths,
and semicolons are safe — no escaping needed. Long messages show up in the
peer's input as a "Pasted text" block but still submit in full.

`send` refuses to message your own pane.

## 3. Read the reply — `domux read [--lines N] [--pane W.P] <worktree>`

```
domux read --lines 80 workspace-2
```

Prints the peer pane's recent output (default 50 lines). Use it to confirm the
peer picked up your message and see what it did.

## Typical handoff flow

```
domux peek                                   # find the peer, check it's idle/waiting
domux send workspace-2 "<the handoff>"       # hand it off (attributed to you)
domux read --lines 80 workspace-2            # later: see that it acted on it
```

## When it can't resolve a name

- *"no worktree or tmux session matches X"* → run `domux peek` for valid names.
- *"X is ambiguous"* → two sessions share that name/label; use the exact session
  name from `domux peek`.
- *"session X has multiple Claude panes"* → add `--pane W.P` (see the target
  column in `domux peek`).
