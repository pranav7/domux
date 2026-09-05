# 0002: How the Ghostty source reaches the build

**Date:** 2026-09-05
**Status:** Accepted
**Decision:** The build clones Ghostty at the commit pinned in `vendor/ghostty-pin.toml` and caches it under `~/.cache/domux/ghostty/<commit>`. The repository holds no Ghostty source. `DOMUX_GHOSTTY_SOURCE_DIR` points the build at a local checkout instead.

## Context

M0 vendored the whole upstream Ghostty repository as a git subtree at `vendor/ghostty`: 5,879 tracked files, about 105 MB. On disk it read as 136 MB, because Zig writes about 31 MB of fetched packages into `zig-pkg/`, which Ghostty's own `.gitignore` excludes.

`crates/domux-term/build.rs` declares its inputs as five paths: `build.zig`, `build.zig.zon`, `src`, `include`, and `pkg`, roughly 1,260 files. The rest is never read by the build. `test/` alone is 4,020 files.

Three facts decided this:

1. **Vendoring was not buying hermetic builds.** `build.zig.zon` declares 16 external dependencies that Zig fetches from `deps.files.ghostty.org` and GitHub at build time, and nested packages under `pkg/` fetch more. A cold build has always needed the network, with or without the subtree.
2. **domux does not patch Ghostty.** The only commit touching `vendor/ghostty` is the subtree import itself. There are no local modifications to preserve.
3. **The repository was taking ownership of a dependency.** Every bump wrote another full-tree delta into history, and no reviewer can read a 5,879-file diff.

## Why a commit and not an archive

A pinned tarball plus a SHA-256 was the obvious alternative. GitHub does not guarantee archive bytes are stable: it documents that "the exact compression settings used to generate a zipball or tarball may change over time" while the extracted contents stay the same. A pinned archive checksum can therefore fail closed on a cold build long after the pin was correct, which forces you to host a mirror of the exact bytes.

A git commit has no such problem. Object names are content addressed, so checking out the commit *is* the integrity check, and git verifies it. There is no second checksum to maintain and no mirror to host. `--filter=blob:none` fetches only the blobs the checkout needs: the pinned commit clones in about 4 seconds.

This is also what the one existing Rust binding to this library does. `libghostty-vt-sys` in [uzaaft/libghostty-rs](https://github.com/uzaaft/libghostty-rs) pins `GHOSTTY_COMMIT`, clones blobless into `OUT_DIR`, and offers `GHOSTTY_SOURCE_DIR` as an override.

## Consequences

- One resolved tree feeds everything. `build.rs` exports it as `DOMUX_GHOSTTY_SRC`, and `tests/header_fingerprint.rs`, `tests/zig_pin.rs`, and `scripts/regen-ghostty-bindings.sh` all read that same tree, so the bindings are always checked against the headers that were compiled.
- Availability moves from git history to the pin. If upstream disappears, an old checkout no longer carries the source. `DOMUX_GHOSTTY_SOURCE_DIR` covers working from a local copy, and anyone who wants a guarantee can keep their own clone and point at it.
- Removing the subtree does not shrink existing history. The whole `.git` is 36 MB and stays that size. What changes is that checkouts stop carrying 5,879 files and future bumps stop adding full-tree deltas.
- A bump is one command: `crates/domux-term/scripts/bump-ghostty.sh <commit>` rewrites the pin, rebuilds, regenerates the bindings, runs the suite, and prints the upstream compare link. The two-line pin diff hides the real review surface, so read that range.
- The build fetches Ghostty's Zig dependencies as it always did. `zig build --fetch` now runs with the same options as the build (`-Demit-lib-vt=true`, `-Demit-xcframework=false`, `-Dtarget=`); without them it resolved a different, larger dependency set than the build needed. Lazy nested packages under `pkg/` still resolve during the build itself, so `--fetch` narrows the parallel fetching rather than eliminating it.
- `deps.files.ghostty.org` returns `TlsInitializationFailed` to Zig intermittently on some networks while plain HTTPS to the same URL succeeds. This predates the change and is why the fetch pass is serialized with `-j1`. A cold build may need a retry.
