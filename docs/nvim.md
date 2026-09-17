# Neovim

With the domux plugin, `C-h`, `C-j`, `C-k` and `C-l` move between Neovim windows, and from the last
window in a direction they move on to the next domux pane or into the sidebar. `C-\` goes back to
the window or pane you were in before. One set of keys crosses both.

Without the plugin, those keys move between domux panes even when Neovim is in front, and Neovim
never sees them.

The plugin needs Neovim 0.10 or later. Decision record
[0054](decisions/0054-a-program-claims-the-focus-keys.md) says why it works the way it does.

## Add the plugin

### lazy.nvim

```lua
{
  "pranav7/domux",
  version = "*",
  lazy = false,
  keys = {
    { "<C-h>", function() require("domux").navigate("left") end, desc = "Window or pane left" },
    { "<C-j>", function() require("domux").navigate("down") end, desc = "Window or pane below" },
    { "<C-k>", function() require("domux").navigate("up") end, desc = "Window or pane above" },
    { "<C-l>", function() require("domux").navigate("right") end, desc = "Window or pane right" },
    { "<C-\\>", function() require("domux").navigate("last") end, desc = "Last window or pane" },
  },
}
```

- `lazy = false` loads the plugin when Neovim starts. domux keeps the keys until the plugin has
  loaded, so a plugin loaded later would make the first `C-h` of a session move a pane instead of
  a window.
- The keys go in `keys` rather than in a keymap file. LazyVim maps `C-h` to its own window move
  and leaves the key alone only when a lazy.nvim `keys` entry has it.
- `version = "*"` takes the latest release, which is the release the installer gave you. A plugin
  newer than your domux binary can call something the binary does not have.
- lazy.nvim clones without file contents it does not need, so the Rust code in the repository
  costs little.

### Other plugin managers

Install `pranav7/domux` the way your manager installs a plugin, and call `setup` once:

```lua
require("domux").setup()
```

`setup` maps the five keys in normal mode. To use other keys, skip `setup` and map
`require("domux").navigate` yourself. It takes `"left"`, `"down"`, `"up"`, `"right"` or `"last"`.

### With tmux too

If you also run Neovim in tmux with vim-tmux-navigator, load each plugin only where it applies,
so the two never map the same key. Add `cond = vim.env.TMUX == nil` to the domux spec, and
`cond = vim.env.TMUX ~= nil` to vim-tmux-navigator's.

## Check that it works

Run `:checkhealth domux` in Neovim. It reports:

- whether Neovim runs in a domux pane, and which one
- whether the domux server answers, and its version
- whether domux accepted the plugin's claim on the keys
- the last move that reached domux, and whether it worked

If a key at the edge of your windows does nothing, press it and run `:checkhealth domux`. No move
listed means the key never reached the plugin, because something else in your configuration maps
it too.

## How it works

The plugin **claims** the keys. When Neovim starts in a pane, the plugin tells domux, and domux
passes `C-h`, `C-j`, `C-k`, `C-l` and `C-\` to Neovim while Neovim is in front of that pane. When
Neovim has no window in the direction you pressed, the plugin tells domux to move focus.

- `C-z` or quitting Neovim puts the shell back in front, and domux takes the keys again at once.
- Neovim started by another program, such as `git commit`, claims the keys the same way.
- A move that arrives after you have already gone somewhere else does nothing.
- While the sidebar has the keys, they are the sidebar's, so `C-l` always comes back out.

## The passthrough list

`[keys.passthrough]` in `~/.config/domux/domux.toml` lists the commands that get the keys by
name, without a claim:

```toml
[keys.passthrough]
commands = ["fzf"]
keys = ["C-h", "C-j", "C-k", "C-l", "C-\\"]
```

`fzf` is there because it moves through its list with `C-j` and `C-k`. Before the plugin, `nvim`
and `vim` were in the list too, and the keys stayed inside them for good. To get that back, add
them:

```toml
[keys.passthrough]
commands = ["nvim", "vim", "fzf"]
```

## For other programs

Any program that can open a Unix socket can do what the plugin does. domux sets `DOMUX_PANE` and
`DOMUX_SOCKET` in every pane. Send one JSON line per request to `DOMUX_SOCKET` and read one line
back:

```json
{"id":1,"method":"pane.claim_passthrough","params":{"pane":"p_8f2a"}}
```

| Method | What it does |
|---|---|
| `pane.claim_passthrough` | Claims the passthrough keys in `pane` for the process on the other end of the socket. The claim holds while that process is in the pane's foreground process group. |
| `pane.release_passthrough` | Gives them back. |
| `focus.left`, `focus.right`, `focus.up`, `focus.down`, `focus.last` | With `pane`, move focus from that pane, and only while it has the keys. |

A claim belongs to the process that opens the connection, so make the call from the program
itself. A claim made with `domux api` belongs to that command, which ends at once.
`domux api schema` prints every method's parameters.
