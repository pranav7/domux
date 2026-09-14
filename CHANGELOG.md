# Changelog

Every release has a section here, newest first, in the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
shape: a heading `## [<version>] - <date>` and, under it, `### Added`, `### Changed`, `### Fixed`
or `### Removed` as the release needs. The release pipeline reads this file: the section for the
version being released becomes the release notes, and a missing or empty section stops the
release. Versions follow [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `domux server upgrade` replaces the running server with a new build and keeps every pane: the
  programs in them go on running, their screens and scrollback come back, and agents keep their
  state, session name and recap. Attached clients leave and attach again on their own. A server
  started before this release cannot hand over, so the first upgrade onto it is a
  `domux server restart`.
- `scripts/dev/deploy.sh` pulls, builds and upgrades the server in one step, for running a build
  of your own checkout.

### Changed

- The install command is `curl -fsSL https://domux.dev/install.sh | sh`. The GitHub URL it
  replaces still works.

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

## [1.0.1] - 2026-09-13

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

Themes, an installer that asks for your leader and stay awake, and fixes for the hooks and for
attach that showed up on Linux. After upgrading, run `domux server restart`: the attach protocol
changed, so a 1.0.0 server and a 1.0.1 client refuse each other. The default leader is now `C-s`.

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

## [1.0.0] - 2026-09-11

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

The first release of the Rust domux. domux V1, the Go version built on tmux, is the v0.x tags;
`domux import v1` reads what it saved.

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

[Unreleased]: https://github.com/pranav7/domux/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/pranav7/domux/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/pranav7/domux/releases/tag/v1.0.0
