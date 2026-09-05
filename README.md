# domux V2

A terminal multiplexer for engineers who direct AI agents. This branch is a prerelease built as `domux2`; V1 lives on `main` until the M3 cut-over.

Build: `cargo build --release`. Test: `cargo test --workspace`. Spec and plans live outside the repository.

The pane emulator is [Ghostty](https://github.com/ghostty-org/ghostty)'s libghostty-vt, and it is the only one: M0 measured it against `alacritty_terminal` and the comparison is recorded in `docs/decisions/0001-terminal-emulator.md`. Its source is not vendored here: the first build clones the commit pinned in `vendor/ghostty-pin.toml` and the Zig that builds it, caching both under `~/.cache/domux`. Set `DOMUX_GHOSTTY_SOURCE_DIR` to build a local Ghostty checkout instead.
