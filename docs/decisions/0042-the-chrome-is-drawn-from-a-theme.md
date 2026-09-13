# 0042: The chrome is drawn from a theme

Date: 2026-09-13
Status: accepted

Every colour the chrome draws is a role, and a theme gives each role a value. Two themes are built in:
`domux`, which is the colours domux has always drawn, and `terminal`, which takes them from the terminal's
background, foreground and palette, asked once at attach, and on Omarchy follows a theme change while
attached. `[theme] name` picks one, or a theme file under `~/.config/domux/themes/`. The default is
`auto`: `terminal` when the client finds Omarchy on a local session, `domux` everywhere else. The agent
kind colours and the band are not taken from the terminal, and under `terminal` they are held to the same
readability floor as text.

This record amends interface spec 9.1, which fixed the chrome's colours; the attach protocol, which goes to
version 4; and AGENTS.md's rules that the dot is red and that the stay awake dot is green or grey (decision
0029), which now hold for the two built-in themes and for any theme that keeps those roles.

`0042-data/` holds the reference model the numbers below came from and the answers of Omarchy's 22 themes.

## Context

The author runs Omarchy with Ghostty, and Omarchy themes the terminal. On Ristretto the terminal
background is `#2c2525`, and the switcher was drawn on `#1e1e2e` with a `#313244` selected row:
Catppuccin Mocha's navy on a warm brown. Pane content already followed the terminal, because a default
colour goes out as `Reset` and an indexed colour stays indexed. Only the chrome was hard-coded, as 26
constants in `render/theme.rs` that 13 other files used 117 times outside their tests.

The client already asked the terminal for its foreground and background at attach, and nothing but the
pane emulators read the answers.

The author asked for more than a colour patch. domux ships its own look, mauve accent and all, and that
look has to stay exactly as it is on macOS and on every system with no theme of its own. Anyone should be
able to write a theme. A few more built-in themes will come later, and this change should lay the ground
for them without shipping them.

## Why roles and not a remap

A remap after the frame is composed, swapping each Catppuccin value for the terminal's, cannot work.
`CODEX` and `BLUE` are the same hex and one of them has to stay a kind colour, and a pane can draw any RGB
value at all. The renderer has to ask for a role.

The role names are the interface a theme author writes against, so they use the words the chrome already
has. `fill` is the selected row, as it is in `list_box`. The grounds are `overlay_background`,
`top_bar_background`, `toast_background`, `sidebar_background` and `tab_row_background`. The last two
name a decision the renderer used to make without a name: the sidebar column and the tab row on the
panes are drawn on the terminal's own ground. Their value is `default` in both built-in themes, so no
frame changes, and a theme may paint them. There are 43 roles, and their names are public from the first
release. Renaming or removing one breaks somebody's theme file and needs a record like this one.

## Why `domux` is a theme and not a fallback

The domux theme is a file in `domux-core`, every role equal to the colour it replaced, and a test lists
every value. It is the root of every theme, so a role nobody set always has a value, and it is what
`terminal` falls back to when the terminal does not answer. Under `domux` nothing is derived and nothing
is adjusted, so its frames are the frames domux drew before this record, on every platform and in every
existing test.

The default is named `domux` rather than `catppuccin`, because it is not Mocha: the branch pink and the
workspace teal are the interface spec's own values, and the accent is the logo's mauve. The author calls
them the domux colours.

A future built-in theme is a file beside it and one line in `BUILTIN`.

## How `terminal` derives its colours

Neutrals blend the terminal's background toward its foreground, in ninths: `fill` and `rule` at 1/9,
`separator` 2/9, `border` 3/9, `faint_text` 4/9, `dim_text` 5/9, `soft_text` 7/9. Those are the fractions
the Catppuccin neutrals already sit at between `BASE` and `TEXT`, exactly, so on a Mocha terminal
`terminal` draws the neutrals `domux` does. The top bar and the toast are a shade: the background moved a
fifth of the way to black on a dark terminal and to white on a light one, which is `MANTLE` exactly on
Mocha. On a dark theme the bar is darker than the overlay, as it has always been, which keeps what is
drawn on it readable.

Colours come from the palette. The accent is slot 4, because Omarchy writes its accent there in 17 of its
22 themes. Hint keys are slot 6, so they never read the accent's slot: the accent means "your input goes
here" and nothing else. Different slots can still be close colours in a given theme, and the guards do
not change that. Branches are slot 5, a merged pull request takes the accent's slot as it always has,
green is slot 2 and red is slot 1. Slots 0, 7 and 8 are never read, because Omarchy writes the
background, the foreground and its muted grey there.

## Why there are guards

Nobody chose the terminal's colours for domux, so two rules hold them to what the chrome needs. They run
only when some role's value came from the terminal, and they never move a hex value a theme file wrote.
That is why `domux`, and a theme written in hex, are drawn exactly as written.

The first is a readability floor, against every ground the role is drawn on. Checking only the overlay was
not enough: a top bar a step lighter than the overlay put the faintest text under 3.0 on 16 of 22 Omarchy
themes. A role under its floor moves in steps of 1/18 toward white on a dark ground and toward black on a
light one. Moving toward the foreground was tried first; a tinted foreground took the colour out of a
moved role, and rose-pine's green stay awake dot came out grey. A theme that extends `terminal` can paint
one ground in hex, a dark sidebar on a light terminal, and then a role sits on grounds on both sides of it.
The first version moved such a role all the way to black or white, where it read worse than unmoved. Now a
move is taken only when it lowers no role's lowest contrast: the fewest steps toward the far end that meet
the floor, else the fewest toward the other end, else the step that reads best on the worst ground, which
may be none.

- Text, the colours, the kind colours and the band ends: contrast 3.0, the WCAG figure for large text and
  marks. The text tiers move together, by the fewest steps that bring every one of them to the floor, so
  they keep their order. The kinds and the band were left alone in the first design, and on
  catppuccin-latte Codex was 1.86 against the background and a band's bright end under 1.5; they now move
  like text and keep their hue. Neither text
  nor the kinds are held to the floor on the selected row, where Mocha's own band dims are under 3.0, so
  a floor there would change the band on the terminal whose colours domux's are.
- The lines: 1.4 for the separator and the border, 1.25 for the rule, 2.0 for the stay awake dot for
  "not held". Before these, the `│` between tabs on rose-pine's top bar, which is the rule, was 1.10 and
  the dot 1.57; they are now 1.41 and 2.26. The rule has the lower floor because it is 1.30 on a Mocha
  terminal and 1.34 on Ristretto, and a floor of 1.4 would have moved it on both. None of the line floors moves anything on Mocha or Ristretto.
- The top bar, the toast and the selected row, when they come from the terminal, are held at least a
  contrast of 1.05 from the overlay, so they show on a black or white background too.

The second is a hue test. The dot is red because it means an agent is waiting on you, and a green pill
means it worked. On vantablack, solitude and white the "red" slot is grey, on lumon it is blue, on
hackerman green. So a red or green role takes its palette slot only when the slot's OKLCH hue is red or
green, before and after any move, and otherwise keeps the value of the theme it extends. Red is a hue from
345° to 40°, green from 115° to 170°, and either needs an OKLCH chroma of at least 0.03, so a grey is
neither.

## Why `auto` asks the client about Omarchy

The author wants Omarchy to get `terminal` and every other system to keep domux's own look, so `auto`
needs to know where it is. The client checks, because the client is started by the reader from the
terminal the chrome is drawn in, while the server may have been started from somewhere else, and two
clients on one server can be in two places. It looks for `OMARCHY_PATH` or Omarchy's
`~/.local/state/omarchy/current/theme.name`, and sends what it found in the hello. `theme::auto` is the
one function that turns that into a theme name, so a per-OS default later is one arm there.

A session over ssh is not Omarchy, whatever the host says. Omarchy exports `OMARCHY_PATH` into login
shells, ssh included, so both signals describe the machine the client runs on, and the reader's desktop is
on the other end of the connection. A reader who attaches over ssh and wants `terminal` names it.

## Why theme files are files

A theme is a thing people pass around, so it is one file, `themes/<name>.toml` beside `domux.toml`, and
`extends` names a built-in theme or another file the same way. A role is a hex colour, `background`,
`foreground`, `palette N`, `blend n/d`, `shade n/d`, or, for a ground, `default`.

Only `auto`, `domux` and `terminal` are reserved. For any other name, a file wins over a built-in of the
same name and a warning says which built-in it hides, so shipping a built-in theme later never changes a
reader's chrome.

A mistake in a theme file is a warning, returned by `config reload` and written to the log. A bad value or
an unknown role drops that role, and the theme it extends supplies it. A file that does not parse, a cycle,
a chain of more than 8 themes, or a name with no file makes the theme unused: at server start `auto` applies, and on `config reload` the
theme drawn before stays, the way a broken `domux.toml` keeps the previous config. A theme name that is not
a name is a warning too, not a config error, so it never costs the reader their key bindings.

## How the terminal is asked

Once, at attach, where the two colours were asked before. The client writes one batch: OSC 10, OSC 11,
OSC 4 for slots 0 to 15, and a device attributes query that ends it. It reads the answers until the device
attributes answer arrives or a cap passes, and the answers go in the hello, so the first frame is already
in the right colours. The cap is 1 s past the time crossterm's keyboard probe took to be answered a moment
before, which a terminal that answers never waits for, because its device attributes answer ends the read;
it is 100 ms, the old deadline, when the probe got no device attributes answer in 2 s. The longer cap is
what keeps colour answers out of the pane: an answer that arrives after the read has ended is read by
crossterm as `Alt+]` and characters and typed into the focused pane, and before this record that happened
to any answer slower than 100 ms. A flat 1 s was tried first, and a terminal that took 2 s over each answer
had all eighteen colour answers typed into the pane; one that is that slow over the probe is as slow over
the batch, so the cap follows the probe, to at most 3 s. A terminal that gives no device attributes answer
and answers colours after 100 ms still has them typed into the pane, all eighteen where the old attach
asked for two. Every terminal domux runs in answers device attributes, so this is left as a limit.

After attach the client asks nothing and reads no answer. Key, mouse and paste input stay crossterm's.

## How the chrome follows a theme change on Omarchy

The server tells each client whether its theme reads the terminal. A client whose theme does, and that
found Omarchy, looks at Omarchy's current theme about once a second: it compares the inode, time and length
of `current/theme.name`, `current/theme/colors.toml` and `current/theme/ghostty.conf`, which
`omarchy-theme-set` replaces on every theme change, the same theme set again included. When they changed,
it reads the background, foreground and 16 palette slots from the theme's files and sends
`ClientMsg::Colors` when they differ from what the files gave the last time they were read, starting from
the read at attach; the server paints that client's chrome again. So a theme left as attach found it sends
nothing, even when the terminal answered a palette slot of its own, and the terminal's answers stand until
the first change. When nothing changed, nothing is read and nothing is sent. A file that cannot be read or
parsed changes nothing, is reported as a warning, and is not read again until it changes. The attach
client has no log file, so that warning reaches no file today; see Consequences.

The colours come from `colors.toml` when it carries all 16 keys Omarchy's terminal templates use, and from
`ghostty.conf` otherwise. `colors.toml` is the theme's own palette and the same for every terminal Omarchy
themes; `omarchy-theme-set-templates` renders `ghostty.conf` from it after filling in whatever keys it
leaves out, so a file that needs that cascade is read in its rendered form rather than domux repeating the
cascade. All 22 shipped themes' `colors.toml` carry every key and give exactly the colours their Ghostty
file does.

The client follows only when Omarchy's file names the background and foreground the terminal answered at
attach. `OMARCHY_PATH` reaches every login shell, so without that check a terminal that does not draw
Omarchy's theme, an editor's terminal for instance, would have its chrome recoloured to a desktop theme it
does not show.

## Protocol version 4

The hello carries the palette and the desktop, and `ClientMsg::Colors` and `ServerMsg::FollowColors` are
new. The colours start with the same two fields the old hello ended with, so an old server reads a new hello
and refuses it with "run domux server restart". A new server that cannot decode an old hello reads its
version and protocol number from the front of the frame and refuses it with the same sentence, rather than
dropping the connection. `Hello` stays the first message variant and starts with the version and the
protocol, forever.

## What was set aside

- **Following a theme change on every terminal while attached.** Set aside for a follow-up issue. It needs
  the client to read the terminal's answers out of its own input, and crossterm types them into the pane,
  so the client would own its input: a thread reading the terminal, and crossterm's parser copied into
  domux (about 860 lines and 650 of tests) with colour answers, the device attributes answer and mode 2031
  reports added. On a terminal without the kitty keyboard flags `ESC ]` is both `Alt+]` and the start of an
  answer, so the parser holds those bytes until the next ones decide, delays a real `Esc` up to 50 ms, and
  still types `Esc` and an answer into the pane when a slow link splits the answer right after its ESC.
  Answers that come after a batch's deadline have to be counted as owed and merged, and a detach while one
  is owed has to wait for it, or the shell's line editor gets them. What starts a batch is weak everywhere
  but Ghostty: mode 2031 reports come on every Ghostty config reload, but not from other terminals, and
  looks after a focus change rest on guessing how long Omarchy's script takes. Omarchy is where the author
  works, and there the theme's files already say what changed, with none of this. Other terminals pick up a
  change at the next attach.
- **`termina` instead of a copied parser.** It does not parse OSC 4, and it reads every `ESC ]` as an OSC
  string, so a real `Alt+]` on a terminal without the kitty flags holds back every key after it until a
  BEL.
- **Asking the terminal again on a timer.** Every ask puts answers into the input stream the client does
  not own.
- **An Omarchy theme-set hook that calls domux.** Installing it writes under `~/.config/omarchy`, outside
  domux's own files, and the files it would react to are already there to read.
- **inotify for the watch.** Three `stat` calls a second cost nothing measurable and need no care when
  `omarchy-theme-set` removes the theme directory and moves a new one in its place, which ends a watch held on
  the old one.
- **Reimplementing `omarchy-theme-color`'s cascade** to read any `colors.toml`. The rendered `ghostty.conf`
  already holds its result.
- **Terminal colours as the default everywhere.** The author's first choice, with Catppuccin as the
  fallback. The brief replaced it: where the desktop ships no theme, domux draws its own look.
- **Naming the default `catppuccin`.** Its pink and teal are not Mocha's, and the author calls them the
  domux colours.
- **Keeping the kind colours and the band fixed under `terminal`.** They meant what they meant on a dark
  terminal only; on the five light Omarchy themes Codex was 1.86 to 2.11 against the background and the
  band's bright ends 1.22 to 1.69.
- **One floor of 1.4 for every line.** It moves the rule on Mocha and Ristretto.
- **Holding text and the kinds to the floor on the selected row.** It moves the faintest text and the band's
  dim ends on Mocha.
- **Inline `[themes.<name>]` tables in `domux.toml`.** A theme is shared as a file; tables stay possible
  later without changing a layer's format.
- **Slot 5 for the accent.** Closer to mauve on Catppuccin, but it leaves Omarchy's own accent unused.
- **A top bar blended toward the foreground.** It lowered the contrast of everything on the bar.
- **Moving a guarded colour toward the foreground.** It took the hue out of red and green roles on themes
  with a tinted foreground.

## Consequences

- A server from before this record refuses a client from after it: `domux server restart` after the
  upgrade.
- On Omarchy the chrome follows a theme change within about a second. Everywhere else, under `terminal`, it
  follows at the next attach.
- A live change does not reach the default colours programs in an existing pane hear for OSC 10 and 11;
  those still come from the most recent client when the pane was made. A pane made after the change gets the
  new ones when that client is the most recent.
- On an Omarchy theme change the chrome can change a few seconds before the panes, because
  `omarchy-theme-set` reloads the terminal after the shell's background transition.
- The watch reads Omarchy's files as Omarchy's scripts lay them out today. The match check that starts
  it compares only the background and the foreground, and every change after it takes the palette from
  the files, so a theme that ships its own `ghostty.conf` with palette slots that differ from a complete
  `colors.toml`, or a Ghostty config that sets palette slots after Omarchy's theme, is followed from the
  files. If Omarchy moves the files, the chrome stops following and the next attach still reads the
  terminal.
- The attach client installs no log, so the watch's warnings, like the client's other warnings, reach no
  file, and a theme file the watch cannot read is seen only as a chrome that did not change. A log of the
  client's own is left open.
- A key typed while attach waits for the answers is read with them and dropped, as it was before this
  record, but the wait is now as long as the terminal takes to answer, up to the cap, rather than 100 ms.
  Over a slow ssh link that is about a round trip. A signal is held and ends the session once it starts.
- A colour answer slower than the attach read is still typed into the pane, as it was before; the read now
  waits 1 s past the keyboard probe's time for a terminal that answers device attributes, rather than
  100 ms.
- On light themes under `terminal` the kind colours and the band are darker than the domux values, and the
  band's moving highlight is faint: its two ends are 1.16 to 1.47 apart.
- The dot is red, and the stay awake dot green or grey, as far as the theme keeps those roles. Both built-in
  themes do.

## What this does not do

- It ships two built-in themes and no gallery, no picker and no per-OS default. The places those would go
  are named above.
- It does not theme pane content. The workpanel is the panes' ground, and programs draw on it.
- It does not watch theme files. A reload applies them.
- It does not follow a theme change live on terminals other than Omarchy's.
