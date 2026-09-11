# 0036: An install follows the agent's configuration directory

**Date:** 2026-09-11
**Status:** Accepted.
**Decision:** `install claude` writes in the directory `CLAUDE_CONFIG_DIR` names when it names
one, and in `~/.claude` otherwise. `--dir <path>` names a directory over both. Codex and
OpenCode follow no variable, because domux has not seen either read one.

## Context

Claude Code keeps everything it reads under one directory, and `CLAUDE_CONFIG_DIR` moves it. A
session started that way reads no `settings.json` under `~/.claude` at all, so the hooks domux
installed there never run for it.

That is not a corner. The author runs Bedrock as a second path to the same agent, through an
alias that sets `CLAUDE_CONFIG_DIR=$HOME/.claude-bedrock` so a Bedrock model id cannot leak into
the subscription configuration. Both paths run `claude`, so the observer sees the process and
makes a record either way, and one of the two had no hooks: the row sat on `unknown` with no
state, no recap and no session name for the whole session, which is exactly what hook loss looks
like. Nothing was broken in the record, the state machine or the reader. The file was in the
other directory.

## The rule

Three sources, in this order:

1. `--dir <path>`, the directory the reader named.
2. The variable the kind declares, when it is set and not empty: `HookTarget::dir_env`.
3. The kind's own directory under `HOME`: `HookTarget::dir_in`.

`agents::install::plan` takes the directory and reads no environment, so a test names the
directory it means and two installs in one test cannot disagree about what the environment said.
The CLI resolves it, next to where it already reads `HOME`.

A hook file is now the kind's file inside a directory rather than one path under home:
`dir_in` and `file` answer the two halves, `path_under` joins them, and `path_in` is the two
together for the default directory.

## Why a variable and not only a flag

A flag alone would mean the author has to remember which of two configuration directories a
shell is for. The variable is already the thing that decides, so an install run from inside a
Bedrock session installs for Bedrock without being told, and one run from an ordinary shell
installs where it always did. The flag is for the third case: installing for a directory you are
not currently running in.

## What this does not do

It does not find every configuration directory on the machine and patch each one. An install
writes one file and says which, and a reader with two directories runs it twice. Walking the home
directory for anything that looks like a Claude configuration would guess, and a hook line in a
file the author did not mean to hand over is worse than a row that reads `unknown`.

It does not change what an install writes. The events, the command, the removal of V1's lines,
the backup and the second run that changes nothing are all as they were. In particular an install
into a directory that still carries V1's `ai-state` lines takes them out, the same as anywhere
else, so a reader who still runs V1 from that directory reads the preview before applying.

Nothing about the record changes. The observer, `transition` and the dot are untouched: this is
only which file the hooks are written into.
