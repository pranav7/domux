# Repository guidelines for domux V2

Rust workspace. Crates live under `crates/`. `domux-term` holds the emulator trait and its implementations; `m0-spike` is the M0 pane spike and will be lifted or deleted in M1.

## Build and test

- `cargo build --workspace --all-features` builds both emulator implementations. The `ghostty` feature downloads a pinned Zig on first build (see `vendor/zig-pin.toml`) and builds `vendor/ghostty`; set `DOMUX_ZIG=/path/to/zig` to use a local Zig of the same version.
- `cargo test --workspace --all-features` runs unit, golden, and behavior tests.
- `UPDATE_GOLDEN=1 cargo test -p domux-term --test golden` rewrites golden files. Review the diff against the fixture's intent before committing.
- `cargo clippy --workspace --all-features -- -D warnings` and `cargo fmt --all --check` must pass before a commit.

## Rules

- Branch `v2` only. Never commit to `main`, `master`, or `workspace-*`.
- Nothing here writes under `~/.local/share/domux`, `~/.config/domux`, `~/.claude`, or `~/.codex`. V1 owns those until the M3 cut-over.
- No tmux. No mouse. No Windows.
- Atomic writes: write `path.tmp`, then rename.
- Test names are `behavior_condition` in snake_case.
- Prose and comments: no em dashes, sentence case, plain words, active voice, one term per concept.
