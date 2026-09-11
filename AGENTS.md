# Repository guidelines for domux V2

Rust workspace, five crates under `crates/`:

- `domux-term`: the `Emulator` trait and `GhosttyEmulator`, its one implementation. There is no second emulator and no cargo feature to select. Read `docs/decisions/0001-terminal-emulator.md` and `docs/decisions/0002-ghostty-source-acquisition.md` before touching this crate.
- `domux-core`: pure model, layout, keymap, config, protocol and state types. No IO, no tokio, no ratatui, no process spawning. Every rule here has a unit test.
- `domux-server`: the core task, panes, rendering, socket, persistence. `src/testing.rs` is the harness every interface test uses.
- `domux-client`: the thin client: raw mode, capabilities, keys out, frames in, clipboard.
- `domux`: the `domux2` binary. One file per API namespace under `src/cli/`.

The M0 pane spike is gone. M1 lifted its PTY, input and render code into `domux-server` and deleted `crates/m0-spike`; `docs/milestones/m1.md` records what M1 shipped.

## Build and test

- `cargo build --workspace` fetches two pinned inputs on the first build and caches both under `~/.cache/domux`: the Ghostty source, cloned at the commit in `vendor/ghostty-pin.toml`, and the Zig that builds it, from `vendor/zig-pin.toml`. `DOMUX_GHOSTTY_SOURCE_DIR` and `DOMUX_ZIG` point at local copies.
- A cold build can fail while Zig fetches Ghostty's dependencies: `deps.files.ghostty.org` returns `TlsInitializationFailed` intermittently on some networks. The fetch pass already runs with `-j1`; rerun the build. Decision 0002 records the details.
- `crates/domux-term/scripts/bump-ghostty.sh <commit>` is the only thing that edits `vendor/ghostty-pin.toml`. It regenerates the bindings and runs the suite.
- `cargo test --workspace` runs everything. Interface tests live in `crates/domux-server/tests/` and assert on frames: a failing test prints the screen as `|...|` rows with a styles list.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` must pass before a commit. CI runs `cargo fmt --all --check`, `cargo build --workspace --locked`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace --locked`, in that order.
- `UPDATE_GOLDEN=1 cargo test -p domux-term --test golden` rewrites the golden files. Read the diff against the fixture's intent before committing it: a golden that changed because the emulator changed is the point, and one that changed because a test was loosened is a defect being recorded as correct.
- `crates/domux-term/scripts/ghostty-src.sh` prints the Ghostty tree the build resolved. `build.rs` exports the same path as `DOMUX_GHOSTTY_SRC`, and `tests/header_fingerprint.rs` and `tests/zig_pin.rs` read it, so the bindings are always checked against the headers that were actually compiled.
- `cargo run -p domux -- api schema` prints the control API schema.
- Run `domux2` only in its own Ghostty tab, never inside tmux. `~/bin/domux` is V1 and is never touched; `~/bin/domux2` points at `target/release/domux2`.

## Agents

M3 added the agent records. `docs/milestones/m3.md` says what shipped and what is still open, and `docs/decisions/0017`, `0019` and `0020` record the choices the code does not explain on its own. `0026` and `0027` record what MUX-18 to MUX-23 changed about the boxes and the transcript reader, and **`0030` replaces a good deal of both**: it combines the two boxes into the Navigator and takes resume, dismiss and the exited record away. Read it first. `0018` is retired with the record it was about.

- One record per AI coding session. `domux_core::model::agent` holds the record and `transition`, a pure function of (state, event), table-tested over every pair. Never add a state or an event without extending that table: a state is a row, an event is a column in every row.
- Two sources write records: hooks, through `agent.report`, and the observer, in `agents::observer`. The observer does three things and only three: it creates an `unknown` record, it binds a process id to a record a hook made, and it ends a record whose own process is gone. It never sets a state, and `transition` says so: `(s, Observed) => s`.
- **A record ends when its session ends, and there is no exited state.** `transition` answers `Option`, where `None` is the record ending, which is what `SessionEnd` and `ProcessGone` mean from every state. Nothing resumes an agent and nothing dismisses one.
- Only `SessionStart` makes a record from a hook. Every other hook that finds none is a message from a session domux is not tracking, and inventing a record for one leaves a row nothing can take away.
- So a record exists whether or not the hooks are installed, and the end is seen either way, but **a lost hook is not made good later**. A `Stop` that never arrives leaves the row `working` with a turning glyph until the process dies. A row stuck on `working` is what hook loss looks like; go and look rather than waiting.
- An agent nobody reports on is `unknown`, never `idle`. A recap that did not arrive is absent. A session name is absent until the agent sets one. A working word is never shown for a state other than `working`.
- A record ends when its own process id goes away, or when its pane goes away. Never because the foreground changed: an agent running a tool puts that tool in front, and the tool can itself be an agent.
- `unseen` turns on when an agent starts waiting or goes from working to idle, which is what `agent::attention` answers. It clears when the agent's pane is focused or when input reaches that pane, and on nothing else. Reading the records does not clear it. All it does now is brighten a recap in the switcher, and whether it earns its keep is an open question 0030 names.
- **A dot is drawn only while an agent is waiting on you, it is red, and it sits two cells after the name** in the slot the working word would take. Working and compacting say themselves with the glyph and the word; idle and unknown draw nothing. The top bar draws no agent count.
- The row grammar lives in one place, `render::agents_box`, and the Navigator draws it through `one_row` rather than writing a second one. The sidebar's row is one line; the switcher's adds the tab and the recap.
- **Nothing reorders the Navigator.** Projects are alphabetical, workspaces keep their order, and agents sit under their workspace in the order they started, so no row moves while a state changes. `Model::sorted_agents` keeps its attention order for `agent.list` and `peek`.
- The recap is the agent's summary of its last turn, and a summary written before the last prompt is not one. The session name is the transcript's last `custom-title`.
- Installers preview by default. `--apply` backs up the file it patches, writes `path.tmp` and renames, and is idempotent. Never run `--apply` against `~/.claude` or `~/.codex` without the author saying so.

## Stay awake

MUX-15 built it as a slice of M4, and `docs/decisions/0029` records the choices the code does
not explain on its own. Read it before changing any of them.

- The feature is **stay awake**, never "stay alive" and never the name of the program that
  implements it. `caffeinate` and `systemd-inhibit` appear in code and in one error each, and
  nowhere a reader meets in normal use.
- A child process holds the machine awake and dies with the server. `stay-awake.pid` beside
  the state file is how a server that crashed finds the hold it left; a process id is only
  adopted when it is both alive and still the program that was started.
- The dot at the right end of the top bar is the state, green for held and grey for not. It
  is drawn outside the right end's priority chain, so nothing the bar says takes it away.
- A toast says what changed. It is the surface M4's Notifier draws into, so it lives in
  `toast.rs` and `render/toast.rs` rather than in the stay awake code.
- Full mode is the lid. On Linux it is one more flag on the same child; on macOS it needs the
  launch daemon and the sudoers line that `stay-awake install --full` writes, and that command
  is the only thing in domux that ever asks for sudo. Never run it with `--apply` without the
  author saying so.

## Rules

- Work on `v2`, or on a milestone branch (`m1`, `m2`, `m3`) that merges into `v2` by pull request. Never commit to `main`, `master`, or `workspace-*`.
- Nothing here writes under `~/.local/share/domux`, `~/.config/domux`, `~/.claude`, or `~/.codex`. V2 uses `~/.local/share/domux2`, `~/.config/domux2/domux.toml` and `domux2.sock` until the M3 cut-over; every such name comes from `domux_core::names` and `domux_core::paths`.
- No tmux. No Windows. The mouse is read: the wheel, a drag that selects, clicks on the
  chrome, and a click that opens the link under it. Read `docs/decisions/0014-the-mouse.md`
  and `docs/decisions/0024-a-click-opens-the-link-under-it.md` before changing what any of
  them do.
- One implementation per operation: a key, a CLI subcommand and an API call reach the same handler in `domux_server::api`.
- One core task owns all mutable state. Atomic writes: `path.tmp`, then rename.
- Test names are `behavior_condition` in snake_case.
- Prose, comments, help and errors: no em dashes, sentence case, plain words, active voice, one term per concept. Use the words of `2026-09-05-domux-v2-domain-model.md` (outside this repository): screen, top bar, tab row, tab, pane, workpanel, sidebar, switcher, agents overlay, overlay, box, accent, focused region, cursor, fill, hint row, footer, prompt. For agents: agent, kind, session id, session name, state, working word, glyph, unseen, dot (never any other word for it), recap, reason, hook, observer, manifest, Navigator, agent row, place. `resume`, `dismiss`, `exited` and the Agents box are retired words: decision record 0030 removed what they named. Never "window" for the screen, "panel" for the sidebar, or "modal", "popup" or "dialog" for an overlay.
- Decisions that the specs left open are recorded under `docs/decisions/`; read them before changing the behaviour they describe. Each milestone's protocol and acceptance criteria are under `docs/milestones/`.
