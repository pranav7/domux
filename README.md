# domux

**A terminal multiplexer that shows which of your parallel AI agents is waiting on you.**

domux is how I run many coding agents at once. Each workspace is a git worktree with its own agent, and one key answers "who needs me right now?". The first version was a layer on top of tmux. This one is a rewrite in Rust that owns the terminal itself, so domux knows the pane, the workspace and the project of every agent, because it handed out that place. I am comfortable working in the terminal, and that's where I wanted this to work. It's simple, it's not trying to do too much, and it's my daily driver. This is my take on an AI IDE built in the terminal.

There are three parts to domux: the Navigator, the agents overlay, and the panes under them.

### Navigator

The Navigator is one list of your projects, their workspaces and the agents running in them. It lives in the sidebar, <kbd>leader</kbd> + <kbd>b</kbd>, and in the switcher, <kbd>leader</kbd> + <kbd>s</kbd>. A workspace row shows its name, its branch and its pull request. An agent row shows what the agent is doing: a turning glyph and the word `working` while it works, a red dot when it's waiting on you, and nothing when it's idle. The switcher adds the agent's recap, the summary it wrote at the end of its last turn, so you know what you're walking back into. Nothing in the list moves when a state changes, so an agent stays where you left it.

It works with Claude Code, Codex and OpenCode. `domux install claude --apply` adds the hooks that report state, and `codex` and `opencode` work the same way. An agent without hooks still shows up, as `unknown`, because domux watches the process.

### Agents overlay

<kbd>leader</kbd> + <kbd>a</kbd> answers the other question: which agent wants me? It lists the agents alone, under their project, with the ones waiting on you first. <kbd>Enter</kbd> takes you to that pane.

### Workspaces

`domux open .` registers the repository you're in as a project. `domux workspace create` adds a workspace: a git worktree on its own branch, in the lowest free slot, with a tab ready to go. `domux workspace clear` resets it to the base branch, and `domux workspace delete` removes the worktree, the branch and the slot. Name a workspace with <kbd>leader</kbd> + <kbd>N</kbd>.

### Panes

<kbd>leader</kbd> + <kbd>t</kbd> opens a tab, <kbd>leader</kbd> + <kbd>\</kbd> and <kbd>leader</kbd> + <kbd>-</kbd> split the pane, <kbd>leader</kbd> + <kbd>z</kbd> zooms it, and <kbd>ctrl</kbd> + <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd> move between panes. The mouse works: the wheel scrolls, a drag selects, a click focuses a pane, and a click on a link opens it. The panes run on [Ghostty](https://github.com/ghostty-org/ghostty)'s terminal emulator library.

### Stay awake

<kbd>leader</kbd> + <kbd>A</kbd> holds the machine awake while your agents work, and the dot at the right end of the top bar is green while it does. On a Mac, `domux stay-awake install --full` sets up the mode that keeps it awake with the lid closed. That command is the only thing in domux that asks for sudo.

### Scripts and agents

Everything the keyboard does, the command line does too. `domux peek` prints every agent with its state and recap, `domux whoami` tells an agent where it is, `domux events` streams what happens, and `domux api schema` prints the whole API. An agent started inside domux is told which pane it's in, so it can use these as well.

### Install

You need macOS or Linux, [rustup](https://rustup.rs) and git. The first build clones Ghostty at a pinned commit and the Zig that builds it, caches both under `~/.cache/domux`, and takes a few minutes.

```
git clone https://github.com/pranav7/domux
cd domux
cargo build --release
```

Put `target/release/domux` on your `PATH`, then run `domux` in a project. Give it its own terminal tab rather than a tmux pane. It asks whether to register the directory as a project, and then you're in. <kbd>leader</kbd> is <kbd>ctrl</kbd> + <kbd>a</kbd> by default, <kbd>leader</kbd> + <kbd>?</kbd> lists the keys, and `~/.config/domux/domux.toml` changes any of them.

The tmux version is on the [`v1`](https://github.com/pranav7/domux/tree/v1) branch.

That's it, feedback welcome!
