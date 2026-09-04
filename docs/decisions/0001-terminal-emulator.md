# 0001: Terminal emulator for domux panes

**Date:** 2026-09-04
**Status:** Accepted
**Decision:** domux panes use ghostty (libghostty-vt). alacritty_terminal stays behind the `alacritty` cargo feature as the measured fallback and is built and tested in CI.

## Context

The M0 spike criteria (architecture spec, Technology section): a pane must render Claude Code, neovim with a colorscheme, htop, and a wide-character and emoji torture file with no visible difference from Ghostty; frames must stay under 16 ms with eight panes running `yes`; idle CPU must be near zero. Both implementations of the `Emulator` trait were built and measured.

## Environment

| Item | Value |
|---|---|
| Machine | Apple M3 Pro, 12 cores, 18 GB |
| OS | macOS 26.6.2 |
| Reference terminal | Ghostty 1.3.1 |
| Window size | 200x50 cells for the performance runs (headless PTY), 100x30 and 120x40 for fixtures |
| TERM inside panes | xterm-256color |
| Vendored Ghostty | 492300cad104195411d12217dd22f1cd05f31376, vendored 2026-09-04 (tip of `main`) |
| Zig | 0.16.0 |
| alacritty_terminal | 0.26.0 |
| termwiz | 0.23.3 |
| ratatui / crossterm / portable-pty | 0.30.2 / 0.29.0 / 0.9.0 |
| Rust | 1.98.1 |
| domux commit | b81e2e3 |

`v1.3.1` was vendored first and rejected: at that tag `include/ghostty/vt/` has no `terminal.h`, `render.h`, or `grid_ref.h`, so the terminal and render-state C API this plan needs does not exist there. The tip of `main` has them and requires Zig 0.16.0. The reference terminal on screen is therefore Ghostty 1.3.1 while the linked library is a later commit; that gap is the reason the fidelity rows below are marked "not measured" rather than compared loosely.

## Fidelity (spike criterion 1: no visible difference from Ghostty)

E = emulator-attributable difference, R = renderer-attributable (ratatui or crossterm; identical for both emulators), P = pass.

The interactive rows need a person at a Ghostty window comparing two tabs; they were not run in this session and are marked "not measured". The automated rows were run.

| Check | ghostty | alacritty | Notes |
|---|---|---|---|
| Claude Code: prompt box, colors, spinner | not measured | not measured | Needs a person at two Ghostty tabs |
| Claude Code: typing, Ctrl-C, paste of three lines | not measured | not measured | Needs a person |
| Claude Code: Shift+Enter inserts a newline | P (by encoding) | E | Automated: with the kitty disambiguate flag pushed, libghostty-vt encodes shift+enter as `CSI 13;2u`; termwiz encodes a plain CR, so the program cannot tell it from enter. `tests/emulator_behavior.rs` pins both |
| Claude Code: resize window while running | not measured | not measured | Needs a person |
| neovim habamax: colors, line numbers, statusline | P | E | Automated as the `nvim-habamax` golden. Text identical; alacritty adds a background run on every erased line (see the background colour erase finding) |
| neovim: cursor block in normal, bar in insert | P | P | Automated: `cursor_shape_follows_decscusr` passes for both |
| neovim: undercurl under a misspelled word | R | R | ratatui has one underline style; every underline variant renders as a plain underline. Shared by both emulators |
| neovim: `:checkhealth` truecolor and key detection | not measured | not measured | Needs a person |
| htop: bars, colors, box drawing | P | E | Automated as the `htop` golden. Same difference as neovim: trailing background runs |
| htop: F-keys F1 to F10 work | P | P | Automated: `control_and_function_keys_use_legacy_encoding_by_default` covers F5; the encoders map F1-F12 |
| torture file: every row identical to Ghostty | P | E | Automated as the `torture-cat` golden: flag width and tab handling differ |
| torture file: wide char at the edge wraps whole | P | P | Automated: the "Wide at edge" line puts a wide grapheme at column 100 of a 100-column pane and both wrap it whole |
| `yes` in eight panes: output looks identical | not measured | not measured | Needs a person; the timing run below is automated |
| Golden suite: fixtures passing / total | 10 / 10 | 4 / 10 | Six fixtures differ; every difference is listed below |
| Behavior tests passing / total | 11 / 11 | 11 / 11 | The shift+enter expectation is per implementation and records the gap |

### Emulator differences found (all E)

Every one of these is a real difference in the emulator, confirmed by a focused test, not a golden that drifted.

1. **SGR attributes dropped.** alacritty_terminal ignores SGR 21 (double underline), SGR 53 (overline), and SGR 5 (blink). Its cell flags have no blink or overline bit. libghostty-vt keeps all three. Fixture: `sgr-attributes`.
2. **Background colour erase.** After an erase with a non-default background set, alacritty_terminal keeps that background on the erased cells; libghostty-vt reports them as the default background. Confirmed directly by feeding `ESC [ 48;2;28;28;28 m ESC [ K` and reading a trailing cell. Explicitly painted cells agree. This shows up as a trailing background run on every erased line of a full-screen program. Fixtures: `nvim-habamax`, `htop`.
3. **Regional indicator flags.** A flag such as the Japan flag is two wide cells (four columns) in libghostty-vt and two narrow cells (two columns) in alacritty_terminal, so a row of flags ends two columns apart. Every other grapheme tested agrees: ZWJ sequences, skin tones, variation selectors, CJK, combining marks, zero-width joiners. Fixtures: `wide-and-emoji`, `torture-cat`.
4. **Horizontal tab.** A tab leaves a literal TAB character in an alacritty_terminal cell; libghostty-vt advances the cursor and leaves spaces, which is what a grid should hold. Fixtures: `cursor-and-erase`, `torture-cat`.
5. **Shift+enter under the kitty protocol.** With the disambiguate flag pushed, libghostty-vt sends `CSI 13;2u` (what kitty specifies, and what makes shift+enter work in Claude Code); termwiz sends a plain CR.

## Performance (spike criteria 2 and 3)

Eight panes at 200x50 in one process, headless under a PTY, release build.

| Measurement | ghostty | alacritty | Pass mark |
|---|---|---|---|
| Frame p50 / p95 / p99 / max us, eight `yes` panes, 30 s | 329 / 345 / 355 / 805 | 245 / 259 / 268 / 597 | p99 <= 16000 (both pass) |
| Frames rendered in 30 s | 27137 | 10497 | report |
| Bytes fed in 30 s (MB) | 1723 | 666 | report |
| Feed p99 us per batch | 14 | 49 | report |
| Flush p99 us | 101 | 102 | report |
| Draw p99 us | 259 | 173 | report |
| Idle CPU percent, eight idle shells, 60 s after 5 s settle | 0.000 (0 ms CPU over 60.5 s, 17 frames) | 0.000 (0 ms CPU over 60.4 s, 13 frames) | <= 0.5 (both pass) |
| Idle CPU cross-check (`ps -o %cpu`) | not measured | not measured | Needs a second terminal during the run |
| Max RSS MB, eight `yes` panes, 10000-line scrollback | 28 | 228 | report |
| Max RSS MB, eight idle shells | 6 | 6 | report |
| Feed throughput MiB/s (`yes` 10 MB, 80x24, criterion) | 102.6 | 163.4 | report |
| Feed throughput MiB/s (`sgr` 10 MB, 80x24, criterion) | 194.4 | 164.8 | report |
| Feed throughput MiB/s (recorded neovim fixture, criterion) | 234.5 | 190.7 | report |
| Snapshot 200x50 us (criterion) | 187 | 60 | report |
| Budget test: feed 10 MB `yes` at 200x50 | 135 ms (74 MB/s) | 92 ms (109 MB/s) | report |
| Budget test: snapshot 200x50 | 285 us | 64 us | report |

Both emulators clear the 16 ms frame mark by a factor of about 45. The loop has no frame timer, so an idle process blocks in `select!` and renders nothing.

## Build and maintenance

| Item | ghostty | alacritty |
|---|---|---|
| Cold build wall time (`cargo clean -p domux-term`, warm Zig cache) | 42 s | 1.2 s |
| Cold build from an empty Zig cache (downloads the 50 MB Zig tarball, fetches Ghostty's Zig packages, builds) | 54.6 s | n/a |
| Warm rebuild after editing `src/lib.rs` | 0.20 s | 0.42 s |
| Static library size | 10.5 MB (`libghostty-vt.a`) | n/a (rlib) |
| `m0-spike` release binary, both features | 3.9 MB | 3.9 MB |
| Cross build x86_64-apple-darwin on the Mac | pass | pass |
| CI: macOS arm64, Linux x86_64, Linux aarch64 | pass | pass |
| Network needed at build time | Zig tarball once, Ghostty Zig packages once, both cached | none |
| Key encoder | built in (`ghostty_key_encoder`) | termwiz, an extra dependency |
| Gaps found | none | blink, overline, double underline, background colour erase, flag width, tab in cell, kitty shift+enter |

Two build facts worth keeping. Zig fetches Ghostty's package dependencies in parallel and the parallel TLS handshakes fail on this machine with `TlsInitializationFailed`; `build.rs` runs `zig build --fetch -j1` first so a serialized pass fills the cache, then builds. Ghostty also enables its xcframework step whenever `xcodebuild` is on PATH, and the Command Line Tools stub fails without full Xcode, so `build.rs` passes `-Demit-xcframework=false`. Both are one line each and are the kind of breakage the plan's risk table anticipated.

## Decision and reasons

domux panes use libghostty-vt. Criterion 1 decided it: alacritty_terminal has five distinct emulator-attributable differences from Ghostty, and two of them are visible in the author's daily programs. The background colour erase difference changes the trailing background of every erased line in neovim and htop, and the kitty shift+enter difference breaks Shift+Enter in Claude Code, which is the tool this multiplexer exists to drive. libghostty-vt has none of these because it is the engine Ghostty itself renders with, and it needs no separate key encoder.

Performance did not decide it: both clear the 16 ms frame mark by roughly 45x and both idle at effectively zero CPU. alacritty_terminal is faster at raw `yes` throughput and at snapshotting; libghostty-vt is faster on styled and real-program input, uses about an eighth of the memory at eight panes with scrollback, and fed 2.6x more bytes in the same 30 seconds.

The Zig build cost is real but bounded: 42 s cold with a warm Zig cache, 0.20 s warm, and it does not re-run when Rust sources change. It built on macOS arm64, macOS x86_64 (cross), Linux x86_64, and Linux aarch64 on the first CI run.

## Consequences for M1

- Default feature: `ghostty`. Fallback feature: `alacritty`, built and tested in CI, not shipped in release binaries.
- Renderer gaps (R rows above) are M1 client work, not emulator work: ratatui collapses every underline variant to a plain underline, so undercurl in neovim renders as a straight underline.
- The vendored Ghostty is a `main` commit, not a release tag. Re-pin to a release tag once one ships the `vt/terminal.h` and `vt/render.h` API, and record the change here.
- The interactive fidelity rows above are still open. They need a person at two Ghostty tabs following the plan's measurement protocol. M1 should close them before the M3 cut-over, since Claude Code behaviour under a real pane is the acceptance bar.
- Budget constants in `tests/budget.rs` set from the measured worst case times 1.5: `FEED_BUDGET_MS = 250` (measured 135), `SNAPSHOT_BUDGET_US = 500` (measured 285).

## Raw data

Stats files from the measurement runs, copied from `/tmp/m0/`:

- `0001-data/ghostty-yes8.json`, `0001-data/ghostty-idle8.json`
- `0001-data/alacritty-yes8.json`, `0001-data/alacritty-idle8.json`
- `0001-data/criterion.txt` (criterion output for both implementations)
