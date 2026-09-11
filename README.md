<p align="left">
  <img width="100" height="100" alt="image" src="https://github.com/user-attachments/assets/5cafbde2-865b-4f5e-889f-d10d9963a472" />
</p>

# domux

**terminal runtime for scaling coding agents.**

domux allows you to scale running multiple coding agents. Parallel work comes from Projects and Workspaces. A Project is a git repo, and Workspaces are long running git worktrees in that repo. Long running means you don't have to manage their lifecycle, and gives you the ability to deploy as many parallel agents as you want. domux currently natively works with Claude Code, Codex and OpenCode. Agent sessions are automatically organised within projects and workspaces, so you can easily peek at which agent is blocked on you.

domux will keep your terminal running in a background server so when you close your laptop you don't lose your work. The multiplexing works similarly to tmux (that's where the name comes from), but is natively built on top of Ghostty. All your tmux shortcuts should work out the box as well.

<img width="1375" height="905" alt="image" src="https://github.com/user-attachments/assets/b3833f16-1c04-407f-9e46-d591122ba043" />

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
```

The script downloads the release build for your machine, checks it against the release
checksums, and puts the `domux` binary in `~/.local/bin`. It then sets up what domux needs:

- **Agent hooks.** An agent reports its state through a hook, so a row can say working, waiting
  or idle rather than unknown. The script installs the hooks for every agent already configured
  on the machine: Claude Code, Codex and OpenCode. Set `DOMUX_HOOKS=no` to skip that.
- **Stay awake.** On macOS it asks whether to set up the mode that keeps the machine awake with
  the lid closed. That one needs sudo, because it writes a launch daemon and a sudoers line.
  Answer no, or set `DOMUX_STAY_AWAKE=no`, and domux still holds the machine awake while the lid
  is open. `DOMUX_STAY_AWAKE=yes` sets it up without asking.

Nothing else is written and no shell startup file is edited. To pick a version or another
directory, set `DOMUX_VERSION=v1.0.0` or `DOMUX_INSTALL_DIR=/usr/local/bin`. Re-running the
command upgrades in place.

## First run

```sh
domux
```

The server starts on the first run and keeps running after you detach, so a closed laptop does
not lose the work. Then:

| Command | What it does |
|---|---|
| `domux` | attach, starting the server if it is not running |
| `domux peek` | every agent: kind, place, state, recap |
| `domux open <path>` | register a directory as a project and switch to it |
| `domux --help` | everything else |

Inside domux the leader key is `ctrl-a`: `leader a` opens the agents overlay, and `leader ?`
lists the keys.

## License

Apache-2.0. See [LICENSE](LICENSE).
