# M3 cut-over

**Date:** 2026-09-11
**What it is:** M3 plan Task 23, and roadmap section 4. The rename that makes the Rust
rewrite `domux`: the binary, the state directory, the config directory and the socket. This
record says what the code half changed, what each machine runs once, and the branch commands.

## What had already happened

The plan's first two steps ran during the M3 week and needed no code: `import v1` read V1's
sessions on 2026-09-09 (`docs/verification/m2-import-v1.md`), and `install <kind> --apply` for
Claude, Codex and OpenCode replaced V1's hook lines on both of the author's machines. The
author has lived in V2 on both since 2026-09-08. So the exit criterion, "the author removes V1
from their day", held before the rename did.

The plan asked for two checks before this task. Decision record 0001 still carries seven
"not measured" cells in its fidelity table, and the Ghostty pin is still a `main` commit. The
author's daily use of Claude Code, editors and pagers in real panes on two machines stands in
for the measured comparison, by their ruling of 2026-09-11 to go straight to release.

## The code half

One pull request into `v2`:

- `crates/domux-core/src/names.rs`: `BIN_NAME`, `STATE_DIR_NAME`, `CONFIG_DIR_NAME`,
  `SOCKET_FILE_NAME` and `SOCKET_DIR_PREFIX` drop the `2`. `OLD_NAME` keeps it, for the
  migration below, and the guard test now fails on the old name anywhere else, tests included.
- `crates/domux/Cargo.toml`: the `[[bin]]` is `domux`.
- 199 lines in 22 Rust files that spelled the old name in a message, a test or a fixture path
  were read and changed, as the guard was written to force.
- `domux_server::migrate` and `carry_over_from_the_old_name` in the server subcommand: on
  start, `state.json`, `state.json.bak`, `pr-cache.json` and `stay-awake.pid` move from
  `~/.local/share/domux2` to `~/.local/share/domux`, and `domux.toml` from `~/.config/domux2` to
  `~/.config/domux`, each only when it exists in the old place and not in the new. V1's
  `sessions/` in the new directory is never touched, the old directory and its log stay, and a
  scratch server under `DOMUX_STATE_DIR` or `DOMUX_CONFIG_FILE` moves nothing. The plan wrote
  the state move as code and the config move as a command; both are code here so the second
  machine needs no hand steps.
- The `SessionStart` context block and the three messaging stubs stop naming M4, which the
  author closed on 2026-09-11 with nothing further built.
- CI triggers on `main` as well as `v2`.

## Each machine, once

After the pull request merges, in the checkout:

```
cargo build --release
domux2 server stop
ln -sf "$PWD/target/release/domux" ~/bin/domux     # it pointed at the V1 Go binary
hash -r
domux server start
domux install claude --apply
domux install codex --apply
domux install opencode --apply
```

The server start moves the state and config; the log says what moved. The three installers
rewrite the hook lines, which carry the absolute path to `~/bin/domux2`; an agent picks the new
lines up at its next session start. When nothing reaches for the old name any more:

```
rm ~/bin/domux2
```

Rollback, at any point: point `~/bin/domux` back at the V1 binary, copy the hook backups the
installers printed over their files, and move the four state files and the config back.

## The branches, once

In the one checkout, `main` is V1 and `v2` is the rewrite. Origin's `main` holds pull request
14, a README edit, that `v2` does not, so the merge comes first:

```
git switch v2 && git merge origin/main      # keep v2's README
git branch -m main v1
git branch -m v2 main
git push origin -u main
git push origin v1
```

Pull request 11 closes on its own. Every branch after this is cut from `main` and lands on
`main` by pull request; `main` stays restricted.

## Record

| Machine | Rename PR merged | Symlink and installers | Old name removed |
|---|---|---|---|
| personal | | | |
| work | | | |

Branch rename run on: _____
