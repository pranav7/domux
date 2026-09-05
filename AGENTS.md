# Repository guidelines for domux V2

Rust workspace. Crates live under `crates/`. `domux-term` holds the emulator trait and its implementations; `m0-spike` is the M0 pane spike and will be lifted or deleted in M1.

## Build and test

- `cargo build --workspace --all-features` builds both emulator implementations. The `ghostty` feature fetches two pinned inputs on first build and caches both under `~/.cache/domux`: the Ghostty source, cloned at the commit in `vendor/ghostty-pin.toml`, and the Zig that builds it, from `vendor/zig-pin.toml`. Set `DOMUX_GHOSTTY_SOURCE_DIR=/path/to/ghostty` to build a local checkout, and `DOMUX_ZIG=/path/to/zig` to use a local Zig of the same version. No Ghostty source lives in this repository; see docs/decisions/0002-ghostty-source-acquisition.md.
- `crates/domux-term/scripts/bump-ghostty.sh <commit>` bumps the Ghostty pin, regenerates the bindings, and runs the suite. Nothing else should edit `vendor/ghostty-pin.toml`.
- `cargo test --workspace --all-features` runs unit, golden, and behavior tests.
- `UPDATE_GOLDEN=1 cargo test -p domux-term --test golden` rewrites golden files. Review the diff against the fixture's intent before committing.
- `crates/domux-term/scripts/ghostty-src.sh` prints the Ghostty tree the build resolved. `build.rs` exports the same path as `DOMUX_GHOSTTY_SRC`, and the header and Zig-pin tests read it, so the bindings are always checked against the headers that were compiled.
- `cargo clippy --workspace --all-features -- -D warnings` and `cargo fmt --all --check` must pass before a commit.
- The pane emulator is ghostty; see docs/decisions/0001-terminal-emulator.md before touching crates/domux-term.

## Rules

- Branch `v2` only. Never commit to `main`, `master`, or `workspace-*`.
- Nothing here writes under `~/.local/share/domux`, `~/.config/domux`, `~/.claude`, or `~/.codex`. V1 owns those until the M3 cut-over.
- No tmux. No mouse. No Windows.
- Atomic writes: write `path.tmp`, then rename.
- Test names are `behavior_condition` in snake_case.
- Prose and comments: no em dashes, sentence case, plain words, active voice, one term per concept.
