# Themes

A theme gives a colour to everything the chrome draws: the top bar, the tab row, the sidebar, the
overlays, the toast. Each thing it colours is a **role**, and a theme gives every role a value. Pane
content is never part of a theme: programs in a pane draw in the terminal's own colours, whatever the
theme says.

Two themes are built in, `domux` and `terminal`, and anyone can write a theme as a file. Decision record
[0042](decisions/0042-the-chrome-is-drawn-from-a-theme.md) says why it works the way it does.

## Choosing a theme

`[theme] name` in `~/.config/domux/domux.toml` picks the theme:

```toml
[theme]
# auto, domux, terminal, or the name of a file under ~/.config/domux/themes/.
# docs/themes.md says how to write one.
name = "auto"
```

- `auto`, the default, picks for each client: `terminal` when the client runs on an Omarchy desktop on
  this machine, and `domux` everywhere else, macOS included.
- `domux` is domux's own colours, on any terminal.
- `terminal` takes the colours from the terminal.
- Any other name is a theme file (see [Theme files](#theme-files)).

`domux config reload` applies a change to the name or to a theme file, and every attached client draws
it on its next frame. Theme files are not watched, the same as `domux.toml`.

### What auto looks for

A client counts as on Omarchy when `OMARCHY_PATH` is set, or when
`~/.local/state/omarchy/current/theme.name` is a file. A session over ssh never counts, whatever the
remote host has: with `SSH_CONNECTION` or `SSH_TTY` set, `auto` picks `domux`, because the desktop in
front of you is on the other end of the connection. To get `terminal` over ssh, name it in the remote
`domux.toml`.

The client decides, not the server, so two clients attached to one server from two places can each get
the theme that suits them.

## The built-in themes

### domux

The colours domux has always drawn: a mauve accent, a pink branch and a teal workspace name over
Catppuccin Mocha's greys. Every role is a hex colour, so nothing is taken from the terminal and nothing is
adjusted, and every frame looks the same on every terminal.

### terminal

The chrome in the terminal's own colours. At attach the client asks the terminal for its background, its
foreground and its 16 palette colours, and the theme builds every role from those answers. Its file is
built into domux:

```toml
# Derives the chrome from the terminal's own colours. Kinds and the band are not set here, so
# they keep the domux values.
extends = "domux"

[roles]
overlay_background = "background"
top_bar_background = "shade 1/5"
toast_background = "shade 1/5"
fill = "blend 1/9"
sidebar_background = "default"
tab_row_background = "default"
rule = "blend 1/9"
separator = "blend 2/9"
border = "blend 3/9"
text = "foreground"
soft_text = "blend 7/9"
dim_text = "blend 5/9"
faint_text = "blend 4/9"
on_accent = "background"
on_pill = "background"
accent = "palette 4"
hint_key = "palette 6"
workspace_name = "palette 6"
branch = "palette 5"
pr_open = "palette 2"
pr_merged = "palette 4"
pr_closed = "palette 1"
pill_ok = "palette 2"
pill_error = "palette 1"
config_error = "palette 1"
question = "palette 1"
waiting_dot = "palette 1"
stay_awake_dot_on = "palette 2"
stay_awake_dot_off = "blend 3/9"
recap = "foreground"
recap_seen = "blend 7/9"
```

The greys sit in ninths between the background and the foreground, the fractions Catppuccin's own greys
sit at, so on a Catppuccin Mocha terminal `terminal` draws what `domux` draws. The accent is palette slot
4, where Omarchy puts its accent colour, so the focused border matches the rest of the desktop. Hint keys
take slot 6, so a key never looks like the accent. The agent kind colours and the band are not set, so
they keep the domux values, held to the readability floor (see [The readability guards](#the-readability-guards)).

A terminal that does not answer gets the domux colours:

| the terminal answered | greys and grounds | colours |
|---|---|---|
| background, foreground and every palette slot | from the terminal | from the palette |
| background and foreground, not every slot | from the terminal | a slot that answered is used; a missing one takes the domux value |
| only one of background and foreground, or neither | the domux theme | the domux theme |

A background and a foreground count only when both arrive and they differ enough to read one on the
other, a contrast of at least 3.0.

## Following a theme change

On Omarchy, a theme that reads the terminal follows a theme change while you are attached. About once a
second the client looks at Omarchy's current theme under `~/.local/state/omarchy/current/`. When it
changed, the client reads the new background, foreground and palette from the theme's `colors.toml`, or
from its `ghostty.conf` when `colors.toml` does not carry all 16 colours, and the chrome is drawn in them
within about a second. When nothing changed, nothing is read.

It follows only when the terminal is drawing Omarchy's theme: when Omarchy's file names the background and
foreground the terminal answered at attach. A terminal on an Omarchy desktop that sets its own colours,
an editor's terminal for instance, keeps the colours it answered with. A file that cannot be read changes
nothing, and the chrome keeps its colours.

Every other terminal, under `terminal` or a theme that reads it, picks up a theme change at the next
attach. Detach and attach again to see it. A theme written all in hex reads nothing from the terminal, so
there is nothing to follow.

## Theme files

### Where they live and which name wins

A theme file is `themes/<name>.toml` beside `domux.toml`, so `~/.config/domux/themes/<name>.toml` by
default. When `DOMUX_CONFIG_FILE` moves the config file, the themes directory moves with it.

A theme name is 1 to 64 characters of lowercase letters, digits, `-` and `_`, starting with a letter or
digit. No file is read for a name that breaks the rule.

- `auto`, `domux` and `terminal` are reserved. They always mean the built-in, and no file of those names
  is read.
- For any other name, a file wins over a built-in theme of the same name, and a warning names the
  built-in it hides. So a built-in theme domux ships later never changes the chrome of someone who already
  has a file of that name.

`extends` looks names up the same way.

### The format

```toml
# Optional. The theme this one starts from; unset roles come from it. Default "domux".
extends = "terminal"

[roles]
# Any role from docs/themes.md. Each value is a hex colour, "background", "foreground",
# "palette N", "blend n/d" or "shade n/d". A ground may also be "default".
hint_key = "palette 12"
```

- The file has two keys: `extends` and the `[roles]` table. Anything else is a warning, and is ignored.
- `extends` names the theme this one starts from: `domux`, `terminal`, or another theme file. A role the
  file does not set comes from that theme. A file with no `extends` extends `domux`, so every theme ends
  at `domux` and a file only names the roles it changes.
- A chain of themes may be 8 long, counting the theme named and `domux`. A longer chain, a cycle, or a name
  with no file and no built-in means the theme is not used.
- Each value in `[roles]` is a string in one of the forms below.

### Colour values

| form | example | means | needs from the terminal |
|---|---|---|---|
| hex | `"#f38d70"` | that colour: `#` and exactly six hex digits, in either case | nothing |
| `default` | `"default"` | the terminal's own ground, drawn as the terminal's default colour; only a ground takes it | nothing |
| `background` | `"background"` | the terminal's background | the background and the foreground |
| `foreground` | `"foreground"` | the terminal's foreground | the background and the foreground |
| `palette N` | `"palette 4"` | the terminal's palette slot N, from 0 to 15 | the background, the foreground and slot N |
| `blend n/d` | `"blend 1/9"` | the background moved n/d of the way to the foreground | the background and the foreground |
| `shade n/d` | `"shade 1/5"` | the background moved n/d of the way away from the foreground: toward black when the background is the darker of the two, toward white otherwise | the background and the foreground |

- A fraction is `n/d` with `d` from 1 to 255 and `n` from 0 to `d`. There is no percent form.
- Words are lowercase, separated by one or more spaces. Spaces around the value are ignored.
- The grounds are `overlay_background`, `top_bar_background`, `toast_background`, `fill`,
  `sidebar_background` and `tab_row_background`. `default` on any other role is a warning, and the role
  is ignored.
- A value the terminal did not answer for cannot be drawn, so the role takes the value of the theme this
  one extends. `domux` sets every role in hex, so every role always has a colour.

## Every role

There are 43 roles. The domux column is the `domux` theme's value, and the terminal column the `terminal`
theme's. A role `terminal` does not set keeps the domux value.

### Grounds

| role | what it colours | domux | terminal |
|---|---|---|---|
| `overlay_background` | every cell of an overlay: the switcher, the agents overlay, Keys, the name box, the confirmation, and their footers | `#1e1e2e` | `background` |
| `top_bar_background` | the top bar's row | `#181825` | `shade 1/5` |
| `toast_background` | the toast's cells | `#181825` | `shade 1/5` |
| `fill` | the selected row in a box | `#313244` | `blend 1/9` |
| `sidebar_background` | the sidebar column, its boxes and its hint row | `default` | `default` |
| `tab_row_background` | the tab row on the panes | `default` | `default` |

`sidebar_background` and `tab_row_background` are the terminal's own ground in both built-in themes, so
the sidebar and the tab row sit on the panes' ground with no background of their own. A theme may paint
them.

### Lines

| role | what it colours | domux | terminal |
|---|---|---|---|
| `rule` | the rule under a project header, the `│` between tabs and the tab row's `…` | `#313244` | `blend 1/9` |
| `separator` | ` · ` and ` › ` between words: in the tab row, the top bar's right end, the hint row, the footer, line 2 of a row, an agent row's tail and copy mode's keys | `#45475a` | `blend 2/9` |
| `border` | an unfocused box's border and flag, the new tab `+`, and the toast's border | `#585b70` | `blend 3/9` |

In the tab row, the `│` between tabs is `rule` and the ` · ` between words is `separator`.

### Text

| role | what it colours | domux | terminal |
|---|---|---|---|
| `text` | ordinary text: the location label, the current tab while keys go elsewhere, session names, typed filter and name text, the Keys overlay's body and headers, confirmation lines, the size notice, the toast's first line | `#cdd6f4` | `foreground` |
| `soft_text` | the clock, the toast's later lines, the Keys legend and `N more` | `#a6adc8` | `blend 7/9` |
| `dim_text` | an unfocused box's title, other tabs, project headers, `main` when not selected, a pull request's title, a draft or unknown pull request, the tab and pane in an agent row's tail, the place on line 2 in the agents overlay, a confirmation's identity line | `#7f849c` | `blend 5/9` |
| `faint_text` | words in the hint row, the footer and the top bar's right end, the `…` that shortens text, empty text, an untouched slot, `unknown`, the `└` corner, the place on line 2 in the sidebar's Agents box, what a confirmation keeps | `#6c7086` | `blend 4/9` |
| `on_accent` | text on the accent fill: the current tab while keys go to a pane, and the tab name prompt | `#1e1e2e` | `background` |
| `on_pill` | text on a pill | `#1e1e2e` | `background` |

### Colours

| role | what it colours | domux | terminal |
|---|---|---|---|
| `accent` | the focused region's border, bold title and flag, and the accent fill on the current tab and the prompt | `#cba6f7` | `palette 4` |
| `hint_key` | a key in the top bar's right end, the hint row, the footer, the Keys overlay, the name box, the confirmation and copy mode | `#89b4fa` | `palette 6` |
| `workspace_name` | a named or live workspace's name | `#93e2d5` | `palette 6` |
| `branch` | a branch name on line 2 | `#e3b4d8` | `palette 5` |
| `pr_open` | an open pull request | `#a6e3a1` | `palette 2` |
| `pr_merged` | a merged pull request | `#cba6f7` | `palette 4` |
| `pr_closed` | a closed pull request | `#f38ba8` | `palette 1` |
| `pill_ok` | an ok pill's ground | `#a6e3a1` | `palette 2` |
| `pill_error` | a refused pill's ground | `#f38ba8` | `palette 1` |
| `config_error` | the config error in the top bar | `#f38ba8` | `palette 1` |
| `question` | a confirmation's question | `#f38ba8` | `palette 1` |
| `waiting_dot` | the dot `◉` while an agent is waiting on you | `#f38ba8` | `palette 1` |
| `stay_awake_dot_on` | the stay awake dot while the machine is held awake | `#a6e3a1` | `palette 2` |
| `stay_awake_dot_off` | the stay awake dot while it is not | `#585b70` | `blend 3/9` |
| `recap` | a recap on a working, waiting, compacting or unseen agent row | `#ddcaf7` | `foreground` |
| `recap_seen` | a recap on a seen agent row | `#a6adc8` | `blend 7/9` |

### Kinds and the band

The working word wears a band, a bright wave that runs from its dim end to its bright end and back.

| role | what it colours | domux | terminal |
|---|---|---|---|
| `claude` | a Claude agent's kind: its glyph while working, and the word `claude` where a row names the kind | `#de7356` | not set |
| `codex` | a Codex agent's kind: its glyph while working, and the word `codex` where a row names the kind | `#89b4fa` | not set |
| `opencode` | an OpenCode agent's kind: its glyph while working, and the word `opencode` where a row names the kind | `#c678b8` | not set |
| `compacting` | the glyph while an agent is compacting | `#afafff` | not set |
| `band_claude_dim` | the dim end of the band on a Claude agent's working word | `#b85e47` | not set |
| `band_claude_bright` | the bright end of the band on a Claude agent's working word | `#ffc9b0` | not set |
| `band_codex_dim` | the dim end of the band on a Codex agent's working word | `#6478a8` | not set |
| `band_codex_bright` | the bright end of the band on a Codex agent's working word | `#c8daff` | not set |
| `band_opencode_dim` | the dim end of the band on an OpenCode agent's working word | `#9f5d93` | not set |
| `band_opencode_bright` | the bright end of the band on an OpenCode agent's working word | `#f0b5e3` | not set |
| `band_compacting_dim` | the dim end of the band on the word `Compacting` | `#6f6fcf` | not set |
| `band_compacting_bright` | the bright end of the band on the word `Compacting` | `#d8d8ff` | not set |

## The readability guards

Nobody chose a terminal's colours for domux, so two rules hold colours taken from the terminal to what the
chrome needs. A hex value written in a theme file is drawn exactly as written, and never moved or refused.
Under `domux`, and under any theme written all in hex, the guards do nothing.

The guards run when some role's value came from the terminal: `background`, `foreground`, `palette`,
`blend` or `shade`. A `default` ground alone does not turn them on. They may move a role whose value came
from the terminal, and a role that keeps the domux value but is drawn on a ground that came from the
terminal. That is how the kind colours and the band are held under `terminal`.

**A readability floor.** A role is held to a contrast floor against every ground it is drawn on. One under
its floor moves in small steps toward white on a dark ground and toward black on a light one, which keeps
its hue, until it reaches the floor.

A role can sit on grounds on both sides of it: a dark `sidebar_background` a theme wrote in hex, say, and
a light background the terminal answered. When no step that way meets the floor on all of them, the role
takes the fewest steps the other way that do. When neither way meets it, the role takes the step that reads
best on its worst ground, which is often no step at all. A move never makes a role harder to read than it
was.

| roles | floor |
|---|---|
| text: `text`, `soft_text`, `dim_text`, `faint_text`, `recap`, `recap_seen` | 3.0 |
| colours: `accent`, `hint_key`, `workspace_name`, `branch`, the pull request and pill colours, `config_error`, `question`, `waiting_dot`, `stay_awake_dot_on` | 3.0 |
| the kinds and the band ends | 3.0 |
| `separator`, `border` | 1.4 |
| `rule` | 1.25 |
| `stay_awake_dot_off` | 2.0 |

- Contrast is the WCAG 2 figure, where 3.0 is the minimum for large text and marks.
- The text roles written as a blend move together by the same number of steps, so dim text stays dimmer
  than soft text.
- Text and the kinds are not held to the floor on the selected row.
- The top bar, the toast and the selected row, when their colour came from the terminal, keep a small
  step from the overlay background, so they still show on a black or white terminal.
- `on_accent` and `on_pill` are not guarded. Under `terminal` they are the background, and the accent and
  the pills are held to the floor against the background, so text on them reads too.

On a Catppuccin Mocha or an Omarchy Ristretto terminal nothing moves. On a light theme the text, the kinds
and the band move toward black, so the band is a quieter wave there.

**Red means red, green means green.** `pr_closed`, `pill_error`, `config_error`, `question` and
`waiting_dot` say something is wrong or waiting, and `pr_open`, `pill_ok` and `stay_awake_dot_on` say
something is well. A `palette N` value on one of them is used only when that slot's colour is red or green,
before and after any move. Otherwise the role takes the value of the theme this one extends. Some themes
put a grey, a blue or a green in the red slot, and the dot stays red on them.

## What a theme never changes

- Pane content. Programs in a pane draw in the terminal's own colours, and a pane box's border sits on the
  terminal's own ground.
- The dimmed screen behind an overlay, and the reversed colours of a caret or a selection.
- Glyphs, the shape and place of the dot, and the layout.

## Errors and warnings

A theme never blocks the config and never shows the config error in the top bar. Its problems are
warnings. `domux config reload` prints each one as `warning: ...`, and the top bar counts them, as in
`config reloaded, 1 warning`. A theme that failed when the server started is in `server.log`.

A bad value or an unknown role drops only that role, and the theme it extends supplies it:

- `themes/ristretto.toml line 9: unknown role acent is ignored`
- `themes/ristretto.toml line 12: roles.fill "blend 10/9" is not a colour: a blend is 0 to 1; the role is ignored`
- `themes/ristretto.toml line 14: roles.text "default" is not a colour for text: only a ground takes default; the role is ignored`

Some problems mean the theme is not used:

- ``themes/ristretto.toml line 3: invalid string; expected `"`, `'`; the theme is not used``, for a
  file that is not TOML
- `theme.name ristreto: there is no themes/ristreto.toml; the theme is not used`
- `themes/night.toml: extends ristretto, and there is no themes/ristretto.toml; the theme is not used`
- `themes/a.toml: extends b, which extends a; the theme is not used`
- `themes/t1.toml: the chain of themes it extends is longer than 8; the theme is not used`

When the theme is not used:

- **When the server starts**, `auto` applies.
- **On `domux config reload`**, the theme drawn before the reload stays, and the rest of the new config
  applies. It is the same as a broken `domux.toml`, which keeps the previous config.

Two more warnings:

- `themes/nord.toml hides the built-in theme nord; rename the file to use the built-in`
- `theme.name "Nord" (line 2) is not a theme name; use lowercase letters, digits, - and _; auto applies`

A name that is not a theme name is a warning and not a config error, so it never costs you your key
bindings.

## Two examples

### The Ristretto look in hex

`~/.config/domux/themes/ristretto.toml` writes out in hex the chrome a terminal in Omarchy's Ristretto
theme gets under `terminal`. Nothing written in hex is moved, so the chrome is these colours on any
terminal. Two things stay the terminal's: the panes, always, and, unless you uncomment the two lines under
the grounds, the sidebar column and the tab row on the panes. On a white terminal those stay white, and
`text` `#e6d9db` on white is a contrast of 1.37, which is hard to read.

```toml
# The chrome of Omarchy's Ristretto theme, written out in hex. The kind colours and the band
# are not set, so they keep the domux values.
extends = "domux"

[roles]
# grounds
overlay_background = "#2c2525"
top_bar_background = "#231e1e"
toast_background = "#231e1e"
fill = "#413939"
# On a terminal that is not Ristretto's brown, paint these too:
# sidebar_background = "#2c2525"
# tab_row_background = "#2c2525"

# lines
rule = "#413939"
separator = "#554d4d"
border = "#6a6162"

# text
text = "#e6d9db"
soft_text = "#bdb1b3"
dim_text = "#93898a"
faint_text = "#7f7576"
on_accent = "#2c2525"
on_pill = "#2c2525"

# colours
accent = "#f38d70"
hint_key = "#85dacc"
workspace_name = "#85dacc"
branch = "#a8a9eb"
pr_open = "#adda78"
pr_merged = "#f38d70"
pr_closed = "#fd6883"
pill_ok = "#adda78"
pill_error = "#fd6883"
config_error = "#fd6883"
question = "#fd6883"
waiting_dot = "#fd6883"
stay_awake_dot_on = "#adda78"
stay_awake_dot_off = "#6a6162"
recap = "#e6d9db"
recap_seen = "#bdb1b3"
```

Then set `name = "ristretto"` under `[theme]` and run `domux config reload`.

### Following the terminal, with two changes

This one starts from `terminal` and changes two roles: keys in the bright blue slot, and a stronger
selected row. Every other role still follows the terminal, and on Omarchy the whole theme follows a theme
change.

```toml
extends = "terminal"

[roles]
hint_key = "palette 12"
fill = "blend 2/9"
```

## Known limits

- **Existing panes keep the default colours they started with.** A program in a pane that asks the
  terminal for its foreground or background (OSC 10 and 11) hears the colours the pane was made with. A
  theme change while attached does not update them; a pane made after the change gets the new ones.
- **On Omarchy the chrome can change a few seconds before the panes.** The chrome follows within about a
  second of the theme's files changing, and Omarchy reloads the terminal only after the background
  transition.
- **A theme that ships its own `ghostty.conf` with different palette slots is followed from
  `colors.toml`.** When its `colors.toml` carries all 16 colours with the same background and foreground,
  the chrome takes the slots `colors.toml` names, even though the terminal draws the ones `ghostty.conf`
  names. None of Omarchy's shipped themes does this.
- **Other terminals follow a change only at the next attach.**
- **Palette slots can be close colours.** The guards keep colours readable and red and green roles red and
  green; they do not keep two roles apart. On some themes the accent and the hint keys, or a branch and a
  merged pull request, look alike. A theme file can set either role.
