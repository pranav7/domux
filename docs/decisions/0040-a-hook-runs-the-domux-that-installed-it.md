# 0040: A hook runs the domux that installed it

**Date:** 2026-09-13
**Status:** Accepted. Replaces the half of M3 plan assumption 16 that chose `~/bin/domux`.
**Decision:** An install writes `~/bin/domux` into the hooks only when that path resolves to the
same file as the binary doing the install. Otherwise it writes the running binary's own path. An
apply says which binary the hooks run, and an install that passed over something at `~/bin/domux`
says so.

## Context

MUX-36. The author installed 1.0.0 on Linux with the curl installer, and Claude Code showed
`unknown command "agent"` on every hook. The installer put domux at `~/.local/bin/domux` and ran
`install claude --apply` with it. But that machine still had V1 at `~/bin/domux`, and the install
preferred `~/bin/domux` whenever the path existed, so every hook line ran V1. V1 has no `agent`
subcommand, prints that error and exits 1, which Claude reports as a non-blocking failure.

The rule came from M3 plan assumption 16: a hook runs an absolute path, because the agent's
environment decides PATH, and the path is the symlink in `~/bin` because it survives a rebuild
that moves the executable. The first half stands. The second half assumed that anything at
`~/bin/domux` was this program. That held on the author's Mac, where the cut-over had pointed the
link at V2, and it does not hold on any machine V1 was installed on, because V1's installer puts
V1 at `~/bin/domux`. It was never about Linux.

Re-running the 1.0.0 install could not repair it. The plan compares the file's lines before and
after, the lines already named `~/bin/domux`, and so the install said "Nothing to change". No
output named the binary, so nothing a reader saw said the hooks ran the wrong program.

## The rule

`agents::install::hook_binary(linked, running)` answers `linked` when both paths canonicalize
and the results are equal, and `running` otherwise. The CLI passes `~/bin/domux` and
`current_exe()`, and reads the environment; the function only reads the file system, the way
decision record 0036 keeps the installer a function of the paths it is given.

So `~/bin/domux` is written when it is this binary, through one link or several, which keeps
the development setup the rule was written for. A V1 binary, a script, a link to any other file,
a copy, a directory, a broken link and a missing path all fall through to the running binary.
Both sides are resolved because macOS answers `current_exe()` with the path the binary was
started by, which can be a link, and Linux answers the resolved path.

Codex and OpenCode get the same answer, because the CLI resolves one binary for every kind.

## What an install says

An apply prints `The hooks run <path>.`, including when it changed nothing. A file whose hooks
run the wrong program looks installed from every other line, so the program is named every time.
When something is at `~/bin/domux` and was not chosen, a preview or an apply that changes the file
also says `<path> is not this domux, so the hooks do not run it.` A broken link counts, which
covers the case M3 left open: a `cargo clean` breaks the link into `target/release`, and an
install then writes whatever path ran it.

## The repair

No change to `is_v2_line` or to how a plan compares lines. A line that names another binary is a
line an earlier install wrote, so an install with the right binary removes all of them and writes
one per event. A reader whose hooks run V1 installs a fixed release and runs
`install <kind> --apply` with it; the curl installer does both. The 1.0.0 binary cannot repair
it, because its install still sees nothing to change.

## Alternatives

- **Always write the running binary.** It breaks the setup the rule was for: run as `domux`
  through `~/bin/domux`, Linux answers `target/release/domux`, and a `cargo clean` breaks every
  hook.
- **Write `~/bin/domux` only when it is a symlink.** The author's V1 at `~/bin/domux` is a
  symlink.
- **Run `~/bin/domux --version` and look for this version.** It runs an unknown program at
  install time and depends on what V1 prints.
- **Have the curl installer pass the path.** It fixes the curl path only, and a reader who runs
  `domux install claude --apply` by hand gets the old answer.
- **Compare device and inode.** The same answer for links, and it also accepts a hard link at
  `~/bin/domux`. Canonical paths are simpler, and a hard link there is not a setup anyone has
  reported.

## What this does not do

It does not look at PATH. The SessionStart context block and the help name the bare `domux`, so
on a machine where V1's `~/bin` comes before `~/.local/bin`, an agent told to run `domux peek`
still runs V1. That is a warning for the curl installer to give, and a separate change.

The development case moves slightly. `cargo run -p domux -- install claude` runs
`target/debug/domux`, which is not the file `~/bin/domux` links to, so the hooks name
`target/debug/domux` and the install says it passed over `~/bin/domux`. Run the install through
`~/bin/domux` to write the link.
