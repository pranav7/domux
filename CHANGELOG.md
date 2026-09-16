# Changelog

Every release has a section here, newest first, in the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
shape: a heading `## [<version>] - <date>` and, under it, `### Added`, `### Changed`, `### Fixed`
or `### Removed` as the release needs. The section opens with one line that says what the
release brings, a short list separated by commas in 50 characters or fewer, with no markdown and
no full stop, and a blank line after it; domux.dev shows that line beside the newest version. The
release pipeline reads this file: the section for the version being released becomes the release
notes, and a missing or empty section stops the release. Versions follow
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `c` in the Navigator makes a workspace in the project of the row under the cursor, and `D`
  deletes the workspace under it, asking first. Both are listed under `?`. The operations
  themselves are unchanged, including the `worktree.conf` setup a new slot gets.

### Changed

- `workspace.delete` takes its workspace as an optional argument, so a key can mean the row
  under the cursor. A call that names none and has no cursor to read is refused rather than
  deleting the workspace the caller is in; `domux workspace delete <ws>` still requires it.

## [0.1.2] - 2026-09-16

Upgrade in place, swap panes, clicks reach Claude

```sh
curl -fsSL https://domux.dev/install.sh | sh
```

After upgrading, run `domux server restart`: a server and a client from different versions
refuse each other, and a server from before this release cannot hand over to
`domux server upgrade`.

### Added

- `domux server upgrade` replaces the running server with a new build and keeps every pane: the
  programs in them go on running, their screens and scrollback come back, and agents keep their
  state, session name and recap. Attached clients leave and attach again on their own. A server
  started before this release cannot hand over, so the first upgrade onto it is a
  `domux server restart`.
- `scripts/dev/deploy.sh` pulls, builds and upgrades the server in one step, for running a build
  of your own checkout.
- `leader {` and `leader }` swap the focused pane with the one before or after it, as tmux's
  `prefix {` and `prefix }` do, and focus goes with the pane. `pane.swap` also takes `left`,
  `right`, `up` and `down`, which have no default key; bind them in `domux.toml`. From a script,
  `domux pane swap <dir>`.

### Changed

- A compacting agent's row draws a still `↓` whose colour breathes, where a working agent's row
  turns the star, so the two tell apart in the Navigator's sidebar too. A theme's `compacting` role
  is the arrow's colour half way through each breath, between `band_compacting_dim` and
  `band_compacting_bright`.
- The install command is `curl -fsSL https://domux.dev/install.sh | sh`. The GitHub URL it
  replaces still works.
- A waiting agent's dot is red until you open that agent's pane and grey once you have, so a
  prompt you have read stops calling you while one that just arrived still does. The dot itself
  stands until the agent is answered, so a row never falls silent while its agent waits, and an
  agent that asks again goes red again. A theme sets the grey with the new `waiting_dot_seen`
  role.
- Versions start at 0.1.0. The releases published as 1.0.0 and 1.0.1 are 0.1.0 and 0.1.1,
  rebuilt from the same code with only the version changed, and the installer and the release
  check accept `v0` tags. domux V1, the Go version, keeps its history on the `v1` branch; its own
  v0.x releases were deleted. Decision record 0048 says why.

### Fixed

- A click in a pane whose program asked for the mouse reaches that program, so Claude Code's own
  interface answers it: the `×` on its sidebar, a click that moves the cursor or expands a result,
  and a drag that selects and copies with Claude Code's own selection. The press, the drag and the
  release all go to the program, and the press still focuses the pane. domux's own drag, double
  click, triple click and link click still work in every other pane. Over such a program, shift and
  a drag reach the terminal's own selection, and `leader [` then a drag uses domux's.
- An agent that another agent starts in its pane, such as a worker run from Claude Code's Bash
  tool, no longer takes over that pane's agent row. The row keeps its session name and its state,
  and the worker gets no row of its own. Restart the server with `domux server restart` for this to
  apply.
- A Codex session renamed with `/rename` shows the new name on its agent row within a second, where
  the row used to say `codex` for the whole session.
- An agent row in the Navigator's sidebar keeps its kind colour after the session is named, so an
  idle row still says which kind is running. The sidebar's row has no second line to name the kind
  on, where the switcher and the agents overlay both do.

## [0.1.1] - 2026-09-13

Themes, a leader you pick, Linux fixes

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

Themes, an installer that asks for your leader and stay awake, and fixes for the hooks and for
attach that showed up on Linux. After upgrading, run `domux server restart`: the attach protocol
changed, so a 0.1.0 server and a 0.1.1 client refuse each other. The default leader is now `C-s`.

### Added

- Themes. `[theme] name` in `domux.toml` picks the colours the chrome draws in: `domux`, the colours
  domux has always drawn; `terminal`, which takes them from the terminal's background, foreground and
  palette; or a theme file under `~/.config/domux/themes/`. The default is `auto`, which draws
  `terminal` on an Omarchy desktop and `domux` everywhere else, and always over ssh. Under `terminal` on
  Omarchy, the chrome follows a theme change while you stay attached.
  [docs/themes.md](https://github.com/pranav7/domux/blob/main/docs/themes.md) says how to choose one
  and how to write one.
- The installer asks you to select your leader, from `C-s`, `C-a`, `C-b` and `C-Space` or any key
  name you type, and writes it under `[keys]` in the config file. `DOMUX_LEADER` answers without
  asking, and without a terminal the installer writes `C-s`.
- The installer asks whether to set up stay awake for a closed lid on Linux as well as macOS. A
  yes writes `mode = "full"` under `[stay_awake]` in the config file, after the sudo setup on
  macOS, and the installer says which keys turn stay awake on inside domux.

### Changed

- The attach protocol is version 4, so run `domux server restart` after upgrading: a server and a
  client from either side of the upgrade refuse each other.
- The default leader is now `C-s`, in place of `C-a`. To keep `C-a`, put `leader = "C-a"` under
  `[keys]` in `~/.config/domux/domux.toml` and run `domux config reload`, or pick `C-a` when the
  installer asks. Running the installer again on a machine that relied on the old default, and
  pressing enter or giving it no terminal, writes `C-s`.
- Pressed twice, the leader sends `C-s` to the pane. A shell at its prompt usually leaves flow
  control on, and there `C-s` pauses the pane's output until `C-q`.
- The installer marks each finished step with a check mark and says what it detected and did,
  with one line per coding agent whose hooks it installed. The release lookup and the download
  turn a spinner while they wait.
- The installer never replaces a leader or a stay awake mode the config file already sets, and
  it writes to the file `DOMUX_CONFIG_FILE` names when that is set. When it replaces a domux
  binary and writes to the config file, it says to run `domux config reload`.
- At attach the client asks the terminal for its whole palette, not only its background and
  foreground.
- `project.add` in the control API refuses a path that is relative, empty or starts with `~`,
  and says what to send instead. It used to resolve such a path against the server's own
  directory. `domux open` and `domux project add` send full paths, so only `domux api` calls and
  key bindings that pass a path meet the refusal.

### Fixed

- Hooks no longer run a V1 domux left at `~/bin/domux`, which made Claude Code report
  `unknown command "agent"` on every hook. An install writes `~/bin/domux` only when it is the
  binary doing the install, and says which binary the hooks run, or the plugin for OpenCode. To
  repair the hooks, run the curl command again, or run `~/.local/bin/domux install claude --apply`
  (and `codex` or `opencode`) once this version is installed.
- `domux attach` in a repository under a folder project, such as a home directory the first
  server was started in, now offers to register the repository instead of saying nothing, and
  `domux open .` registers the directory it was typed in rather than the server's. A home
  project registered by accident is dropped with `domux project remove`.
- `domux attach` in a worktree of a registered repository, or in a submodule checked out in
  one, no longer asks to register it, wherever the worktree was made, and an offer that fails
  says why in one line and attaches anyway instead of ending the command.

## [0.1.0] - 2026-09-11

First release: panes, the Navigator, agent records

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

The first release of the Rust domux. domux V1, the Go version built on tmux, lives on the `v1`
branch; `domux import v1` reads what it saved.

### Added

- A terminal multiplexer with its own core, built on the Ghostty terminal library rather than on
  tmux: a server that owns the panes and outlives the client, tabs, panes, splits, zoom and
  scrollback, and a client that holds the terminal.
- Projects and workspaces: a project is a git repository, a workspace is a long running git
  worktree in it. `domux open` registers a directory, and the Navigator lists every project,
  workspace and agent in one place.
- Agent records for Claude Code, Codex and OpenCode: one record per session, with its state, its
  place and the recap the agent wrote. `domux peek` prints them, `leader a` opens the agents
  overlay, and a red dot marks an agent that is waiting on you.
- `domux install <agent> --apply` installs the hooks an agent reports through, following the
  agent's own configuration directory.
- Stay awake: domux holds the machine awake while it runs, and on macOS
  `domux stay-awake install --full --apply` extends that to a closed lid.
- The mouse: the wheel, a drag that selects, clicks on the chrome, and a click that opens the
  link under it.
- A control API every key and CLI subcommand reaches, printable with `domux api schema`.
- `domux import v1`, which creates what V1's saved sessions describe.
- Release builds for macOS (Apple silicon and Intel) and Linux (x86_64 and arm64), and a curl
  installer that verifies the checksum and sets up the hooks.

[Unreleased]: https://github.com/pranav7/domux/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/pranav7/domux/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/pranav7/domux/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pranav7/domux/releases/tag/v0.1.0
