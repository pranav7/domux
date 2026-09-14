# 0049: A Codex session name comes from its session index

**Date:** 2026-09-14
**Status:** Accepted. Adds a second name source beside the transcript's `custom-title` of
decision record 0034. Claude's name is read as before.
**Decision:** A Codex agent row shows the name the session was given with `/rename`. The core's
once-a-second tick reads it from `session_index.jsonl` in the Codex home, found from the rollout
path the hooks send, and takes the newest line for the record's session id. Each manifest says
where its kind's session name comes from, apart from where its recap comes from.

## Context

MUX-44. The author renamed a Codex session with `/rename babysit-1244`. Codex's own footer showed
the new name, and the agent row under the workspace still said `codex`. Only Claude had a name
source: `recap::poll` read the name from the transcript alongside the recap, and it read only
transcripts whose manifest declared a Claude recap. Codex declares no recap, so nothing ever asked
where its name was.

The issue offered a fallback: if domux could not follow Codex's own rename, it should ship a
rename of its own for agent rows. Codex's rename can be followed, so the fallback is not built.
A second way to name a session would leave two names that can disagree, and the row would have
to choose one.

## Where Codex keeps the name

Read from the Codex source at `rust-v0.154.0`, the version installed here. `/rename` sets the
thread's name through the thread store, which does two things:

- It appends `{"id": <thread id>, "thread_name": <name>, "updated_at": <time>}` to
  `$CODEX_HOME/session_index.jsonl`. The file is append-only, and Codex resolves a thread's name
  by scanning it from the end for the newest line with that id. It does this in both of its
  history modes.
- It writes the name to the `threads` table in `state_5.sqlite`: to the `name` column in its
  paginated history mode, and in its legacy mode to the `title` column, which also holds the
  title Codex makes from the first prompt.

A clear writes a line with an empty name. Deleting a thread rewrites the index without that
thread's lines, through a temporary file and a rename.

The rollout does not hold the name. Older rollouts carried a `thread_name_updated` event, and
Codex's own migration now skips it as a retired record.

## Why the index

- **It is what Codex reads.** Codex resolves a name from the index in its legacy mode, and keeps
  writing to the index in its paginated mode, where the table is its first answer. The index
  tracks the name in both modes.
- **It reads the way a transcript reads.** One file, appended to, one JSON object a line. The
  transcript reader's byte cursor moves to `agents::tail`, and both readers use it, so a read
  takes only the lines Codex appended since the last one and a file that sits still costs one
  `stat`. A rewrite after a delete is the shorter or replaced file the cursor already starts
  again on.
- **The database costs more than it gives.** Reading `state_5.sqlite` needs a SQLite dependency
  in the server, has to share a file Codex holds open in WAL mode, and ties domux to a table Codex
  migrates on its own schedule. In the legacy mode the table can't tell a rename from the title
  Codex made up, and a made-up title is not a session name.

## Why the path comes from the transcript

The index lives in the Codex home, and a session started with `CODEX_HOME` set has its home
somewhere other than `~/.codex`. The server's environment is not the agent's, so the server can't
read that variable, and the record holds no environment. It does hold the rollout path, which
Codex's hooks send as `transcript_path` and which sits under `sessions/` or `archived_sessions/` in
that home. `session_index::index_for` takes the parent of the nearest folder with either name. A
path of any other shape gives no index, and the row shows the kind.

`hooks::parse_codex` read the path from `rollout_path`, the name M3 planned on. Codex sends
`transcript_path`, as Claude does, so no Codex record held a path until now. The adapter reads
`transcript_path` and still accepts `rollout_path` when a payload has no `transcript_path`.

## Rules the reader follows

- **The newest line for the session id is the name.** A root thread's id is the `session_id`
  its hooks send, so the record already holds the key.
- **A blank name is skipped.** Codex writes one for a clear, and the transcript reader already
  skips an empty `custom-title` (principle 4). A name stands until the agent sets another.
- **A name is written only when one was found**, as with the transcript: a read that finds none
  is not evidence that the agent cleared it.
- **One cursor per index, not per record.** Every session under a home shares the index, so no
  single record's end is the time to drop it. Each tick keeps the cursors that some live record
  still reads and drops the rest.
- **A name change marks the frame for a redraw.** A `/rename` sends no hook and a name is not a
  transition, so without that the row went on saying `codex` until something else changed on the
  screen.

## Consequences

- A Codex row shows its name within a second of the rename. It also shows the name a resumed
  session already had, because the first read of the index covers the lines written before the
  session started.
- The first read of an index longer than `tail::TAIL_BYTES` starts that far from the end, as a
  transcript's does. At about a hundred bytes a line, a rename more than twenty thousand renames
  back is outside the window.
- A clear in Codex leaves the old name on the row until the session ends.
- A subagent thread's rename is not followed. Codex's hooks send the root thread's id, and a row
  is a session, not a thread.
- `NameSource` sits beside `RecapSource` on the manifest. Claude declares its transcript for
  both, Codex declares the index for its name and nothing for its recap, and OpenCode declares
  neither.
