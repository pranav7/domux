# Repository guidelines for domux V2

Rust workspace, five crates under `crates/`:

- `domux-term`: the `Emulator` trait and `GhosttyEmulator`, its one implementation. There is no second emulator and no cargo feature to select. Read `docs/decisions/0001-terminal-emulator.md` and `docs/decisions/0002-ghostty-source-acquisition.md` before touching this crate.
- `domux-core`: pure model, layout, keymap, config, protocol and state types, and the theme model: roles, colour values, chains of themes and the readability guards. No IO, no tokio, no ratatui, no process spawning. Every rule here has a unit test.
- `domux-server`: the core task, panes, rendering, socket, persistence. `src/testing.rs` is the harness every interface test uses.
- `domux-client`: the thin client: raw mode, capabilities, keys out, frames in, clipboard, the attach batch that asks the terminal for its colours, the desktop check and the Omarchy watch.
- `domux`: the `domux` binary. One file per API namespace under `src/cli/`.

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
- The shell half has its own suites, all POSIX sh: `sh tests/license/run.sh`, `sh tests/release/run.sh` and `sh tests/install/run.sh` (and the same under `TEST_SHELL=bash`). `shellcheck -s sh install.sh scripts/release/*.sh scripts/dev/*.sh tests/lib/assert.sh tests/license/run.sh tests/release/run.sh tests/install/run.sh tests/install/fakebin/*` must pass too, the same list `.github/workflows/ci.yml` gives it: leave out `tests/lib/assert.sh` and shellcheck exits 1 on the files that source it. CI runs all of it on macOS and Ubuntu.
- `cargo about generate --fail -o /dev/null about.hbs` checks that every compiled crate's license is in `about.toml`. Install it with `cargo install cargo-about --version 0.9.2 --locked --features cli`; without `--features cli` nothing is installed.
- Releases: a `v*` tag runs `.github/workflows/release.yml`, which builds four archives with `scripts/release/build-archive.sh`, smoke-tests each on its own platform, verifies `SHA256SUMS`, and publishes the GitHub release with the `CHANGELOG.md` section for that version. The tag must equal `[workspace.package].version`. Versions stay low: the releases start at 0.1.0, and the next one is the next patch unless the author asks for another. A `CHANGELOG.md` section opens with a one-line summary, which domux.dev shows beside the newest version. `gh workflow run release.yml -f version=<x.y.z>` runs everything except the publish. Read `docs/decisions/0039`, `0048` and `docs/milestones/m5.md` before changing any of it.
- `site/` is domux.dev. `.github/workflows/pages.yml` publishes it with `install.sh` at `/install.sh` on every push to `main` that changes either, so the install command is `curl -fsSL https://domux.dev/install.sh | sh`. The page is one HTML file with no build step. Decision `0047` records why.
- Run `domux` only in its own Ghostty tab, never inside tmux. `~/bin/domux` points at `target/release/domux`. V1, the Go version, is the `v1` branch and is never built or run from here.
- `scripts/dev/deploy.sh` runs the latest build without ending a pane: it fast-forwards the checkout (`--no-pull` builds it as it is), builds release, links `~/.local/bin/domux` and `~/bin/domux` to the build, and runs `domux server upgrade`. It is safe to run from inside a pane.

## Upgrades

`domux server upgrade` replaces the running server with the binary that ran it, by `exec` in the
same process, and every pane keeps running. `docs/decisions/0046` records the design; read it
before changing any of this.

- What crosses is the handover in `handoff/` under the state directory: each pane's master
  descriptor and process id, each pane's screen as a Ghostty snapshot, the agent records with
  their process ids, and the listening socket. `state.json` carries the structure, as at any start.
- `upgrade::HANDOFF_FORMAT` names the handover's shape. A change an older build cannot read bumps
  it, and a server refuses to hand over to a binary that reads another format.
- A pane's PTY is `pane::UnixPty` whether it was spawned or adopted. Its reader stops only between
  two reads, so a byte is either sent to the core or still in the PTY.
- An upgrade never ends a pane for want of something it could do without: a screen that cannot be
  restored gives an empty screen and a resize, and records that cannot be read are dropped.
- A client detached with `SERVER_UPGRADING` replaces itself with `<argv[0]> attach --after-upgrade`.
- `Harness::upgrade` runs the whole handover in the test process with `FakeExec`; only the exec is
  left out. `crates/domux/tests/upgrade.rs` runs the real one.

## Agents

M3 added the agent records. `docs/milestones/m3.md` says what shipped and what is still open, and `docs/decisions/0017`, `0019` and `0020` record the choices the code does not explain on its own. `0026` and `0027` record what MUX-18 to MUX-23 changed about the boxes and the transcript reader, and **`0030` replaces a good deal of both**: it combines the two boxes into the Navigator and takes resume, dismiss and the exited record away. Read it first, then `0033`, which gives `leader a` back: the agents overlay opens in either layout. `0034` then replaces the recap half of `0027`. `0018` is retired with the record it was about.

- One record per AI coding session. `domux_core::model::agent` holds the record and `transition`, a pure function of (state, event), table-tested over every pair. Never add a state or an event without extending that table: a state is a row, an event is a column in every row.
- Two sources write records: hooks, through `agent.report`, and the observer, in `agents::observer`. The observer does three things and only three: it creates an `unknown` record, it binds a process id to a record a hook made, and it ends a record whose own process is gone. It never sets a state, and `transition` says so: `(s, Observed) => s`.
- **A record ends when its session ends, and there is no exited state.** `transition` answers `Option`, where `None` is the record ending, which is what `SessionEnd` and `ProcessGone` mean from every state. Nothing resumes an agent and nothing dismisses one.
- Only `SessionStart` makes a record from a hook. Every other hook that finds none is a message from a session domux is not tracking, and inventing a record for one leaves a row nothing can take away.
- So a record exists whether or not the hooks are installed, and the end is seen either way, but **a lost hook is not made good later**. A `Stop` that never arrives leaves the row `working` with a turning glyph until the process dies. A row stuck on `working` is what hook loss looks like; go and look rather than waiting.
- **A nested agent is not the pane's agent.** An agent that another agent started, such as a worker run from a Bash tool, inherits the pane and reports from it. `agent.report` walks up from the process on the other end of the socket, and a hook with two agents above it, counted below the server, changes no record (`0045`, `agents::nested`). Without that check, the two sessions trade the row on every hook.
- An agent nobody reports on is `unknown`, never `idle`. A recap that did not arrive is absent. A session name is absent until the agent sets one. A working word is never shown for a state other than `working`.
- A record ends when its own process id goes away, or when its pane goes away. Never because the foreground changed: an agent running a tool puts that tool in front, and the tool can itself be an agent.
- `unseen` turns on when an agent starts waiting or goes from working to idle, which is what `agent::attention` answers. It clears when the agent's pane is focused or when input reaches that pane, and on nothing else. Reading the records does not clear it. All it does now is brighten a recap in the switcher, and whether it earns its keep is an open question 0030 names.
- **A dot is drawn only while an agent is waiting on you, it is red under both built-in themes and any theme that keeps `waiting_dot` red, it is `◉`, and it sits one cell after the name**, in the column the glyph takes (`0042`). Working and compacting say themselves with the glyph and the word, or with the glyph alone in the sidebar's two forms (`0038`); idle and unknown draw nothing. The top bar draws no agent count. What turns `waiting` on is a Claude notification that stops for you: `0037` lists the five types, and any other type changes nothing.
- The working word wears a **band**, a bright wave that runs along it and back, carried over from V1 (`render::shimmer` and `theme::Shimmer`). One counter drives it and the glyph both: the band moves every tick, 70 ms since `0038`, and the glyph turns every second one. Read `docs/decisions/0032` before changing either rate, and `0019` for why the ticker never stops.
- The row grammar lives in one place, `render::agents_box`, and the Navigator draws it through `one_row` rather than writing a second one. The sidebar's row is one line; the switcher's adds the tab, pane name and recap. The agents overlay names the pane after the tab too; decision 0035 records why.
- **Nothing reorders the Navigator.** Projects are alphabetical, workspaces keep their order, and agents sit under their workspace in the order they started, so no row moves while a state changes. `Model::sorted_agents` keeps its attention order for `agent.list` and `peek`.
- **Two surfaces, two questions.** The Navigator says where an agent is and never reorders. The agents overlay, under `leader a` in either layout, says which agent wants you: the agents alone, under a header per project, in `Model::sorted_agents` order. `0033` records it as an experiment the author is running, so weigh it on which surface gets opened.
- **A recap is an entry the agent wrote as a recap**, an `away_summary` or a `/recap`, and never the last thing the agent happened to say. The last one stands until the agent writes another. The session name is the transcript's last `custom-title`. `0034` records all of it and replaces the recap half of `0027`.
- **Nothing but the tick reads a transcript.** The recap lands minutes after the hook that ends the turn, so `agent.report` reads no file and `recap::poll` asks once a second. A read takes only the bytes the agent appended, so the cost does not follow the size of the file.
- Installers preview by default. `--apply` backs up the file it patches, writes `path.tmp` and renames, and is idempotent. Never run `--apply` against `~/.claude`, `~/.claude-bedrock` or `~/.codex` without the author saying so.
- **An install follows the agent's configuration directory**: `CLAUDE_CONFIG_DIR` when it names one, `--dir` over that, `~/.claude` otherwise. `docs/decisions/0036` says why. A Bedrock session runs `claude` with that variable set, so it reads no file under `~/.claude`: hooks installed there leave its row on `unknown` for the whole session. `agents::install::plan` takes the directory and reads no environment.
- **A hook runs the binary that installed it**: `~/bin/domux` only when that path resolves to the running binary, and the running binary's own path otherwise. `docs/decisions/0040` says why. V1 installed itself at `~/bin/domux`, and a hook that runs V1 fails on every event with `unknown command "agent"`. `agents::install::hook_binary` takes both paths and reads no environment.

## Stay awake

MUX-15 built it as a slice of M4, and `docs/decisions/0029` records the choices the code does
not explain on its own. Read it before changing any of them.

- The feature is **stay awake**, never "stay alive" and never the name of the program that
  implements it. `caffeinate` and `systemd-inhibit` appear in code and in one error each, and
  nowhere a reader meets in normal use.
- A child process holds the machine awake and dies with the server. `stay-awake.pid` beside
  the state file is how a server that crashed finds the hold it left; a process id is only
  adopted when it is both alive and still the program that was started.
- The dot at the right end of the top bar is the state, green for held and grey for not under
  both built-in themes and any theme that keeps `stay_awake_dot_on` green and
  `stay_awake_dot_off` grey (`0042`). It is drawn outside the right end's priority chain, so
  nothing the bar says takes it away.
- A toast says what changed. It is the surface M4's Notifier draws into, so it lives in
  `toast.rs` and `render/toast.rs` rather than in the stay awake code.
- Full mode is the lid. On Linux it is one more flag on the same child; on macOS it needs the
  launch daemon and the sudoers line that `stay-awake install --full` writes, and that command
  is the only thing in domux that ever asks for sudo. Never run it with `--apply` without the
  author saying so.

## Themes

MUX-38 draws the chrome from a theme. `docs/decisions/0042` records the choices the code does
not explain on its own, and `docs/themes.md` is what a theme's author reads. Read both before
changing any of it.

- Rendering reads roles, never colour constants. A new colour is a new role, with a decision
  record.
- Role names are public: a theme file writes them. Renaming or removing one needs a record.
- Under the `domux` theme every frame is the frame domux drew before themes, and no guard runs.
- A hex value a theme file writes is never moved. The guards move only colours read from the
  terminal's answers, and the built-in `domux` values, such as the kind colours, drawn on a
  ground read from them.
- The terminal is asked once, at attach, in one batch that a device attributes query ends. The
  client never reads the terminal's answers while a session runs.
- Live following is Omarchy's alone. The client watches `current/theme.name`, `colors.toml` and
  `ghostty.conf` under `~/.local/state/omarchy` for a change, reads the colours from `colors.toml`
  or `ghostty.conf` under `current/theme/`, and never writes there. Other terminals under `terminal` pick up a change at the next attach.
- `auto` is decided per client, from that client's desktop, and over ssh it is `domux`.

## Rules

- Work on `v2`, or on a milestone branch (`m1`, `m2`, `m3`) that merges into `v2` by pull request. Never commit to `main`, `master`, or `workspace-*`.
- The state directory is `~/.local/share/domux`, the config file `~/.config/domux/domux.toml` and the socket `domux.sock`; every such name comes from `domux_core::names` and `domux_core::paths`. `sessions/` under the state directory is V1's and is only ever read, by `import v1`. Nothing here writes under `~/.claude` or `~/.codex` except `install <kind> --apply`, and never without the author saying so. The cut-over of 2026-09-11 renamed all of these from `domux2`; `names::OLD_NAME` and `domux_server::migrate` are what carry a machine across it, and no other source spells the old name.
- No tmux. No Windows. The mouse is read: the wheel, a drag that selects, clicks on the
  chrome, and a click that opens the link under it. A program that asked for the mouse gets
  the buttons as well as the wheel. Read `docs/decisions/0014-the-mouse.md`,
  `docs/decisions/0024-a-click-opens-the-link-under-it.md` and
  `docs/decisions/0044-a-program-that-asked-for-the-mouse-gets-the-buttons.md` before changing
  what any of them do.
- One implementation per operation: a key, a CLI subcommand and an API call reach the same handler in `domux_server::api`.
- One core task owns all mutable state. Atomic writes: `path.tmp`, then rename.
- Test names are `behavior_condition` in snake_case.
- Prose, comments, help and errors: no em dashes, sentence case, plain words, active voice, one term per concept. Use the words of `2026-09-05-domux-v2-domain-model.md` (outside this repository): screen, top bar, tab row, tab, pane, workpanel, sidebar, switcher, agents overlay, overlay, box, accent, focused region, cursor, fill, hint row, footer, prompt. For themes: theme, role, ground, answers. For agents: agent, kind, session id, session name, state, working word, band, glyph, unseen, dot (never any other word for it), recap, reason, hook, observer, manifest, Navigator, agent row, place. `resume`, `dismiss` and `exited` are retired words: decision record 0030 removed what they named. The Agents box is the box inside the agents overlay, and the sidebar's Agents box is the one that goes with `[navigator]`. Never "window" for the screen, "panel" for the sidebar, or "modal", "popup" or "dialog" for an overlay.
- Decisions that the specs left open are recorded under `docs/decisions/`; read them before changing the behaviour they describe. Each milestone's protocol and acceptance criteria are under `docs/milestones/`.
