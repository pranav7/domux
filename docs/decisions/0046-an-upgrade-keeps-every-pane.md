# 0046: An upgrade keeps every pane

**Date:** 2026-09-14
**Status:** Accepted.
**Decision:** `domux server upgrade` replaces the running server with the binary that ran the
command, in the same process, by `exec`. Every pane's PTY, its program, its screen and its
scrollback, the agent records and the listening socket cross over. Attached clients leave and
come back on their own. A restart is still what `server restart` does, and it still ends every
pane.

## Context

MUX-46. Running the latest build meant `domux server restart`, and a restart ends every pane:
the shells, the agents in the middle of a turn, their screens and scrollback, and every agent
record's state, session name and recap. The state file brings the structure back and nothing
else. So deploying a build was something the author put off until nothing was running, and a
contributor who wanted to try their own change on their own sessions had the same choice.

## Three ways to keep a pane

1. **A process per pane that holds the PTY**, as dtach and abduco do, with the server as a
   client of those processes. The server could then stop and start freely. It costs a process
   for every pane, a hop through that process for every byte a pane prints, and a second
   protocol between the holder and the server that has to stay compatible for ever, because a
   holder started by one build is read by every later one. It also moves the question rather
   than answering it: the holder cannot be upgraded without the same problem.
2. **Hand the PTYs to a second server over the socket** (`SCM_RIGHTS`), and let the first one
   exit. The programs in the panes would then not be the new server's children, so it could not
   wait for them, and a pane's exit status would be lost.
3. **`exec` in place.** The process id does not change, so every program in a pane is still
   this process's child, and a descriptor without close-on-exec is still open in the new image.
   Nothing new runs while the server is up; the cost is the handover, and it is paid only when
   an upgrade happens.

domux does the third.

## What crosses

| What | How |
|---|---|
| Each pane's PTY | The master descriptor, with close-on-exec cleared, and the child's process id. The new server wraps the two in the same handle a spawned pane gets. |
| Each pane's screen and scrollback | A Ghostty snapshot (`ghostty_snapshot_encode`), restored with the snapshot decoder. |
| The structure | `state.json`, written at once rather than after the persistence debounce. |
| Agent records | The model's `agents`, as JSON, and each record's process id beside it: a record's own JSON leaves the id out, because it means nothing to a later start. |
| The listening socket | The descriptor, so a connection made during the handover waits in the kernel's backlog. |
| The stay awake hold | Nothing new: the holder is still this process's child, and a start already adopts it through `stay-awake.pid` (decision 0029). |

The handover is a JSON file, `handoff.json`, and one screen file per pane, in `handoff/` under
the state directory. The directory is private and the new server removes it once it has read
it: a screen file holds whatever a pane showed.

## What does not cross

- **Attached clients.** Their connections close. Before that the server sends each one
  `Detached` with the reason `the server is upgrading`, the way `the server stopped` works, and
  a client that reads it restores the terminal and replaces itself with `domux attach` from its
  own command line. That is the new binary, so a client never has to speak an older protocol to
  a newer server. The screen flickers once. The reattach asks nothing: it does not offer to
  register the directory.
- **Event subscriptions.** `events.subscribe` connections end, as they do on a stop.
- **Copy mode, a selection, a toast and a pill.** They belong to a view, and the views are gone.
- **Working words.** A working agent draws a new one.
- **The transcript reader's place.** The first poll reads each transcript from the start, and
  the recap it finds is the one the record already carries.

## The sequence

1. **Refuse** when an upgrade is already under way, when the binary is not an executable file,
   or when it reads a different handover format from the one this server writes. The CLI that
   calls `server.upgrade` is the new binary, so the format it names is the one the new server
   reads, and a mismatch is found before the old server has changed anything.
2. **Drain.** Stop accepting connections. Start no job and no fact fetch. Wait until the jobs
   and fetches in flight have finished and every control connection but the caller's has closed,
   or ten seconds, whichever is first. A job that has not finished by then leaves a process
   nobody waits for, which costs a zombie and nothing else.
3. **Pause the readers.** Each pane's reader thread stops after the chunk it is reading and says
   so. Nothing the program prints from then on is read: it stays in the PTY for the new server.
4. **Write the handover**: the screens, `state.json` and `handoff.json`.
5. **Detach the clients**, and wait for their last message to be written.
6. **`exec`** the binary with `server run --handoff <path>`.

If the `exec` itself fails, the old server takes its panes back from the handover it wrote,
starts accepting again, and answers the caller with the error. The clients have already left,
and they reattach to the server that is still there.

The new server starts the way any server starts, from `state.json`, with two differences. A pane
the handover names is adopted rather than spawned, and the handover's agent records go into the
model before anything mints an id.

## When part of it cannot be read

An upgrade never ends a pane for want of something it could do without.

- A screen that cannot be restored leaves the pane with an empty screen, and the new server
  resizes the PTY one row and back, so a program that redraws on a resize redraws. The toast
  says how many screens were lost. The Ghostty snapshot format has no compatibility guarantee
  yet, so an upgrade across a Ghostty pin bump is the likely way to get here.
- Agent records that cannot be read are dropped and the log says why. The observer makes an
  `unknown` record for each agent still running, as it does at any start.
- A pane in the handover that `state.json` no longer holds is hung up and closed. A pane in
  `state.json` that the handover does not name is spawned, as at a start.

What does end every pane is the new binary crashing before it has adopted them, which is what a
restart does anyway.

## Consequences

- The first upgrade onto this build is a restart: a server built before it has no
  `server.upgrade` to call, and the CLI says so rather than printing the unknown method.
- `server upgrade` with no server running starts one, so the deploy script does not have to ask
  which case it is in.
- A stop keeps the socket accepting until its file is removed. The core used to own the listener
  task only through the server handle; now it hands the task back when it stops, because a
  listener that closed first let a restart's new server bind the path and then lose its file.
- The CLI knows the upgrade worked when `server.info` answers with an `upgraded_at` it did not
  answer with before. The process id is the same, so it cannot tell by that.
- `scripts/dev/deploy.sh` is how the author and a contributor run their build: it fast-forwards
  the checkout, builds release, points `~/.local/bin/domux` at the build, and upgrades the
  server, starting one when none is running.
- Ghostty's snapshot needs the parser's unfinished input when a pane is paused in the middle of
  an escape sequence, so every pane's terminal tracks it, up to a limit. A pane past the limit
  at the moment of the upgrade loses its screen, as above.
