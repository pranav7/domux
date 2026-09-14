# 0048: The releases start at 0.1.0

**Date:** 2026-09-14
**Status:** Accepted. Replaces the first decision in 0039, "The public version is 1.0.0", and the
refusal of v0 tags that came with it.
**Decision:** domux's releases are numbered from 0.1.0. The two releases published as 1.0.0 and
1.0.1 are rebuilt from the same commits as 0.1.0 and 0.1.1, and the next release is 0.1.2. V1's
seven releases and their tags are deleted, and the `v1` branch holds V1's last commit. The
release pipeline and the installer take any version tag, v0 included.

## Context

Decision 0039 made the first Rust release 1.0.0 so that the v0.x tags the Go version had
published could stay where they were. The pipeline refused a v0 tag and the installer skipped v0
releases, so the two versions never met.

The author wants low version numbers while domux is young. Two releases in, the attach protocol
has already changed once and the default leader has moved, which is not what a 1.0 promises.
The numbers are cheap to change now and expensive after ten more releases.

V1's releases were v0.1.0 to v0.1.4, v0.2.0 and v0.3.0. Their archives had 11 downloads in all,
every one of them of v0.1.0. `AGENTS.md` already named V1 as the `v1` branch, but no such branch
existed on GitHub.

## Decisions

**V1's releases are retired, and its code is the `v1` branch.** The seven releases and their
tags are deleted. Every commit they tagged is an ancestor of `main`, so no history goes with
them. The `v1` branch points at 81a012d, "UI Fixes", the last commit with V1's Go sources and
the parent of "Start V2", the commit that removed them. That tree keeps V1's own MIT license.

**0.1.0 and 0.1.1 are rebuilt, not retagged.** The binary compiles its version in, and the
archives are named `domux_<version>_<os>_<arch>.tar.gz`, so a new tag on the old commit would
publish archives and a `domux --version` that still say 1.0.x. Each release is a branch instead,
`release/0.1.0` from the 1.0.0 commit and `release/0.1.1` from the 1.0.1 commit, with one commit
on top. That commit sets the workspace version and `Cargo.lock`, lets `check-version.sh` take a
v0 tag, changes that script's test to match, and renames the changelog sections. The code that
runs is the code that shipped. The tag sits on that commit, off `main`, and the pipeline builds,
smoke-tests and publishes it the way it did the first time. The 1.0.0 and 1.0.1 releases and
tags are deleted once both rebuilt releases are published.

**The next release is the next patch.** 0.1.2 follows 0.1.1 unless the author asks for another
number, and a new feature or a breaking change does not raise the minor or the major on its own.
The changelog says what changed. Step 1 of `.claude/commands/release.md` says the same.

**Any version tag passes.** `check-version.sh` takes `v<major>.<minor>.<patch>`, with or without
a prerelease suffix, and still refuses a tag that is not a version. The installer's lookup takes
every tag that starts with `v` and a digit, and a `DOMUX_VERSION` of v0 installs. Once V1's
releases are gone there is nothing to keep apart.

**A release section opens with one line.** domux.dev shows a line beside the newest version that
says what the release brings, such as "Themes, a leader you pick, Linux fixes". The page reads
the newest release from GitHub's API already (decision 0047), and a release does not deploy the
page, so the line comes from the release notes, which are the changelog section. The top of
`CHANGELOG.md` and step 3 of the release command give the rule: a short list separated by
commas, 50 characters or fewer, no markdown, no full stop, and a blank line after it. The line is
the first one in the section, after the blank line under the heading, so the release body starts
with a blank line and a reader of it takes the first line that is not empty.

## Consequences

- The names v0.1.0 and v0.1.1 now mean two things. GitHub has only the Rust releases, but the Go
  module proxy and the checksum database keep their record of `github.com/pranav7/domux` at every
  version they have seen, V1's v0.1.x among them, and they never drop one. Fetching that module
  at v0.1.0 through the proxy still gives V1's Go code, and fetching it directly fails the
  checksum. Nothing in domux is a Go module, so this reaches only someone who imported V1.
- A clone made before the change still has V1's tags and the 1.0.x tags, and a plain fetch
  refuses to move a tag it already has. `git fetch origin --prune --prune-tags --force` makes the
  clone's tags match GitHub's, and the release command runs it before it compares versions.
- A machine on 1.0.0 or 1.0.1 that runs the install command again gets 0.1.1. The server it left
  running refuses the new client at attach, as it does after any change of version, with "the
  server is domux 1.0.1 and this client is 0.1.1; run domux server restart". The restart ends
  every pane: a 1.0.x server cannot hand over (decision 0046).
- A `domux --version` of 1.0.0 and one of 0.1.0 are the same program, and so are 1.0.1 and
  0.1.1. A report that names 1.0.1 is a report about 0.1.1.
- The order of the public steps is fixed by the installer. The installer on domux.dev has to
  take v0 tags before any exists, V1's releases go before 0.1.0 is tagged so the lookup never
  finds one, and 1.0.0 and 1.0.1 go only after 0.1.1 is published, so there is always a release
  to install. GitHub orders releases by the date of the tagged commit, and the 0.1.1 commit is
  the newer of the two, so 0.1.1 is the latest release.
- The install blocks in the 0.1.0 and 0.1.1 sections keep the raw GitHub URL they were released
  with, as decision 0047 says.
- Decision records 0039, 0040, 0041, 0043 and 0047 say 1.0.0 and 1.0.1 where they describe what
  happened then, and they stay as written. So do the protocol fixtures that hold a 1.0.0 client's hello byte for
  byte.
