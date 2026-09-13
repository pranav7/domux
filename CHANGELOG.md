# Changelog

Every release has a section here, newest first, in the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
shape: a heading `## [<version>] - <date>` and, under it, `### Added`, `### Changed`, `### Fixed`
or `### Removed` as the release needs. The release pipeline reads this file: the section for the
version being released becomes the release notes, and a missing or empty section stops the
release. Versions follow [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Themes. `[theme] name` in `domux.toml` picks the colours the chrome draws in: `domux`, the colours
  domux has always drawn; `terminal`, which takes them from the terminal's background, foreground and
  palette; or a theme file under `~/.config/domux/themes/`. The default is `auto`, which draws
  `terminal` on an Omarchy desktop and `domux` everywhere else, and always over ssh. Under `terminal` on
  Omarchy, the chrome follows a theme change while you stay attached.
  [docs/themes.md](https://github.com/pranav7/domux/blob/main/docs/themes.md) says how to choose one
  and how to write one.

### Changed

- `project.add` in the control API refuses a path that is relative, empty or starts with `~`,
  and says what to send instead. It used to resolve such a path against the server's own
  directory. `domux open` and `domux project add` send full paths, so only `domux api` calls and
  key bindings that pass a path meet the refusal.
- The attach protocol is version 4, so run `domux server restart` after upgrading: a server and a
  client from either side of the upgrade refuse each other.
- At attach the client asks the terminal for its whole palette, not only its background and
  foreground.

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

[Unreleased]: https://github.com/pranav7/domux/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/pranav7/domux/releases/tag/v1.0.0
