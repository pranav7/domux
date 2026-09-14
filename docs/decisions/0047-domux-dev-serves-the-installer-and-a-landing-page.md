# 0047: domux.dev serves the installer and a landing page

**Date:** 2026-09-14
**Status:** Accepted. Amends 0039, which served `install.sh` from the raw URL on `main`.
**Decision:** GitHub Pages serves domux.dev from this repository. `.github/workflows/pages.yml`
publishes `site/` with `install.sh` beside it, so the install command is
`curl -fsSL https://domux.dev/install.sh | sh` and the root of the domain is a one-page site that
points to the repository.

## Context

The install command was `curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh`,
which is long to read out and to paste into a message. The author owns domux.dev, registered at
Namecheap.

A `.dev` domain only works over HTTPS: the whole top level domain is on the browsers' HSTS preload
list. Namecheap's URL redirect record serves no certificate, so a redirect set up there fails
before it redirects. The name needs a host that holds a certificate for it.

## Decisions

**GitHub Pages, deployed from a workflow.** Pages issues the certificate, costs nothing and needs
no account beyond the one the repository already has. The DNS stays at Namecheap: four A records
for the apex and a CNAME for `www`. A redirect rule at Cloudflare would also have worked, but it
moves the DNS and keeps the installer's URL in an account's settings instead of in this
repository.

**One `install.sh`.** The workflow copies the file at the repository root into the site on every
push to `main` that changes it. Nothing commits a second copy, and the raw URL keeps working for
anyone who has the old command. Pages caches a file for ten minutes, so a change to the installer
reaches domux.dev within a few minutes of the deploy finishing.

**The page lives in this repository.** Projects with a documentation site that has its own build
often give it a repository of its own; Ghostty, Helix and Ratatui do. Starship keeps its site and
its `install.sh` in the main repository and serves the script from its domain, which is the shape
here. `site/` is one HTML file, a favicon and a screenshot, with no build step, and the page and
the installer it names change together in one pull request.

**The page says what domux is and where to get it.** A top row with the name, a link to the
changelog and a link to the repository with its star count; a heading and a paragraph that say
what domux is and why to use it; the install command with a copy button; the latest version and
its one-line summary, linked to its release notes; a screenshot; six features under it, each with
a mock of the rows domux draws; and a short list of the rest. The layout follows hunk.dev's first
screen: a left-aligned heading in a large bold monospace face, boxes with square corners and a
hard offset shadow, and the screenshot inside a window frame. The colours are the Catppuccin
Mocha ground and mauve the built-in theme draws, and two things on the page are drawn the way
domux draws them: the last word of the heading is a focused tab, the accent as its ground, and
the install command sits in a focused box, the accent as its border. The shadows are the accent
at 30 percent.

The logo's glyph is repeated at an eighth of the accent, with one glyph lit, and the pattern
shows only around the lit glyph, so no text sits on it. The lit glyph is beside the heading on a
screen wider than 64rem and above it on a narrower one. The name is the block logo the installer
prints, drawn as an SVG with each half of a cell as one square, so it matches the terminal
without depending on how a font draws block characters. The text is JetBrains Mono from Google
Fonts, falling back to the system's monospace font.

**The heading sells the idea and the paragraph says what domux is.** The heading is "Run all
your coding agents from one terminal": one idea, the size of it, and the place. The paragraph
opens with "domux is a terminal multiplexer for AI coding agents", a sentence of the shape an
assistant quotes when it recommends a tool, and "terminal multiplexer" is the name comparisons and
searches give this kind of tool; the rest of it is verbs the reader will do, the way tmux's own
first paragraph reads. The page never calls domux a runtime: herdr, the terminal tool of this kind
with the most stars, calls itself one, and a heading that said so read as a copy of it. The title,
the description and the card a shared link unfolds into name the category and all three agents,
which the heading does not, so a search for an agent's name still finds the page.

**The copy names what domux does for the reader, never what it is built on.** Ghostty and Rust
are in the repository and nowhere on the page: a reader deciding whether to install domux asks
what happens to their agents, not which library draws the cells. "Your tmux keys" stays because
the keys are the reader's habit, not domux's implementation.

**Six features, in the order the alternatives sell them, kept to what domux does.** The tools
that compete for the same reader sell parallel agents in worktrees, knowing which agent needs
input, work that survives the lid, one key to switch, an API, and reviewing diffs before a merge.
domux can show the first five and not the last, so the page shows five and says nothing about
diffs, and leads with the three things none of the alternatives state: state reported by the
agent's own hooks, the recap before you switch, and a worktree that arrives with the reader's
`.env` and setup in it. Each feature has a heading of a few words, two or three sentences, and a
mock, the shape hunk.dev uses.

**The mocks are HTML, in the theme's own colours.** A screenshot goes stale with every change to
the chrome and blurs on a phone; a mock is text, so it stays sharp and is edited with the copy.
Each one draws the rows domux draws, from the row grammar in `render::agents_box` and
`render::projects_box` and the roles in the built-in theme, so a reader who installs domux sees
what the page showed. The mocks are hidden from a screen reader: every fact in one is also in the
paragraph beside it.

**What crawlers read is also on the page.** The JSON-LD block describes domux as a
SoftwareApplication, and every fact in it is in the page's text too, because an assistant that
fetches a page reads its text rather than its markup. `robots.txt` lets every crawler in and
names `sitemap.xml`. There is no `llms.txt`: a study of 300,000 domains found it made no
measurable difference to how often an assistant cites a site.

**The star count and the version are read in the reader's browser.** The link to the repository
shows the count and the release link shows the latest release's tag, both from GitHub's API and
fetched on each visit. A value written into the page at deploy time would be out of date until
the next push, and a release does not deploy the page. GitHub allows each address 60
unauthenticated requests an hour, which one reader opening the page twice a visit never reaches,
and a request that fails shows no count or version rather than a wrong one. The release link
shows the first line of that release's notes when the line stands alone, and `CHANGELOG.md` asks
every release to open with one; notes that open with anything else leave the link saying "Read
the release notes".

**No CNAME file.** A site deployed by a workflow takes its custom domain from the repository's
Pages settings and ignores a CNAME file.

## Consequences

- Pages has to be turned on with GitHub Actions as its source before the deploy job can run, and
  the custom domain is set there before the DNS records point at GitHub, the order GitHub's
  documentation gives. Neither is in the repository.
- The domain is verified on the author's GitHub account, so no other account can point a Pages
  site at it.
- `tests/install/run.sh` checks that the README, the page, the installer's header and the release
  check all give the domux.dev command and none still gives the raw URL.
- The sections of `CHANGELOG.md` for 1.0.0 and 1.0.1 keep the raw URL they were released with.
- `og.png` draws the heading over the top of the screenshot, so a new heading or screenshot means
  drawing the card again.
- The repository's description on GitHub is the paragraph's first sentence, and its homepage is
  domux.dev, so a search result for the repository says what the page says.
