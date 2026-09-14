<p align="left">
  <img width="100" height="100" alt="image" src="https://github.com/user-attachments/assets/5cafbde2-865b-4f5e-889f-d10d9963a472" />
</p>

# domux

**Run Claude Code, Codex and OpenCode in parallel.**

domux (_/ˈduː.mʌks/_) is an open-source terminal multiplexer for AI coding agents, for macOS and Linux. Put each agent in its own git worktree, see which one is waiting on you, and keep them all running after you close the lid.

- **Agents in parallel, a worktree each.** A project is a git repo, and each workspace is a long running git worktree in it. Long running means you don't manage their lifecycle, and you can run as many agents side by side as you want without them sharing a checkout.
- **See which agent is waiting on you.** Agents are organised under their project and workspace in the Navigator, which shows what each one is doing, as its own hooks report it, and jumps you to any of them. A red dot marks the one that needs you, and `leader a` lists agents by who needs you first.
- **Close the lid, keep the work.** A background server holds every pane, so closing the terminal or the laptop doesn't lose your work. Turn on stay awake and the agents keep working with the lid closed.
- **Your tmux keys, on Ghostty.** domux ships its own multiplexer, built natively on Ghostty's terminal library rather than on tmux (that's where the name comes from). Your tmux shortcuts work out of the box.
- **Recaps and session names.** For Claude Code, an agent's row shows its session name and the last recap it wrote.
- **Hooks set up for you.** The installer finds Claude Code, Codex and OpenCode and sets up the hooks each one reports through.

<img width="1375" height="905" alt="image" src="https://github.com/user-attachments/assets/b3833f16-1c04-407f-9e46-d591122ba043" />

## Install

```sh
curl -fsSL https://domux.dev/install.sh | sh
```

## First run

```sh
domux
```

The server starts on the first run and keeps running after you detach, so a closed laptop does
not lose the work. Then:

| Command             | What it does                                       |
| ------------------- | -------------------------------------------------- |
| `domux`             | attach, starting the server if it is not running   |
| `domux peek`        | every agent: kind, place, state, recap             |
| `domux open <path>` | register a directory as a project and switch to it |
| `domux --help`      | everything else                                    |

Inside domux the leader key is the one the installer asked for, `ctrl-s` unless you picked
another: `leader a` opens the agents overlay, and `leader ?` lists the keys.

## License

Apache-2.0. See [LICENSE](LICENSE).
