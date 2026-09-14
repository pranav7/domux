<p align="left">
  <img width="100" height="100" alt="image" src="https://github.com/user-attachments/assets/5cafbde2-865b-4f5e-889f-d10d9963a472" />
</p>

# domux

**Run all your coding agents from one terminal.**

domux (_/ˈduː.mʌks/_) is a terminal multiplexer for AI coding agents, with a map of every one you run. Give each agent its own git worktree, see which one is waiting on you, and close the lid while the rest keep working. Your tmux keys, on macOS and Linux.

- **A worktree for every agent.** A project is a git repository, and a workspace is a worktree in it on its own branch. `.domux/worktree.conf` links in what a fresh checkout lacks, such as `.env` and `node_modules`, and runs your setup. Start an agent in each workspace and no two share a checkout.
- **The one that's waiting on you.** Each agent reports its own state through its hooks, so a row says working, waiting or idle without guessing from the screen. A red dot marks the agent that stopped for you, and `leader a` lists agents by who needs you first.
- **Read the recap before you switch.** A Claude Code row carries the session's name and the last recap the agent wrote. The switcher and `domux peek` show both, so you know what an agent did before you go and look.
- **Close the lid. Nothing stops.** A server owns every pane, so detaching or closing the terminal stops no agent, and you attach again from any terminal, over ssh included. Stay awake keeps the machine up with the lid shut; the installer offers to set it up.
- **Your tmux keys, and your mouse.** Tabs, splits, zoom, scrollback and copy mode answer the keys you already know, behind a leader you pick at install. The wheel scrolls, a drag selects, a click opens the link under it, and a program that asked for the mouse gets it.
- **Scriptable by you, and by your agents.** Every key, subcommand and API call reaches the same handler, so anything you can press you can script: `domux events` streams what happens as JSON, and `domux pane send-text` types into a pane. Every agent starts with a note saying where it is and how to ask domux who else is running.

domux is its own multiplexer, so there is no tmux to install; the name is a nod to where its keys come from.

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
