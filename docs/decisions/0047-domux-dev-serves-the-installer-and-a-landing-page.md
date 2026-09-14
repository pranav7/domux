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

**The page says what domux is and where to get it.** The name, the tagline, the install command
with a copy button, the agents and platforms it works with, a link to the repository and a
screenshot. The background is the logo's glyph repeated at a quarter of the accent, with one
glyph lit, in the Catppuccin Mocha ground and mauve the built-in theme draws. The name is the
block logo the installer prints, drawn as an SVG with each half of a cell as one square, so it
matches the terminal without depending on how a font draws block characters. The text is
JetBrains Mono from Google Fonts, falling back to the system's monospace font.

**The star count is read in the reader's browser.** The link to the repository shows the count
from GitHub's API, fetched on each visit. A count written into the page at deploy time would be
out of date until the next push. GitHub allows each address 60 unauthenticated requests an hour,
which one reader never reaches by opening a page, and a request that fails shows no count
rather than a wrong one.

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
