---
description: Cut a domux release: check the state, write the changelog, tag, watch the pipeline
---

Release domux $ARGUMENTS (a version such as 1.0.1; ask for one if it is missing).

Work through these in order and stop at the first thing that does not hold. Every failure names
what to do next; do that rather than working around it.

## 1. The version is a real next version

- `git status --short` is empty and the branch is the one releases come from.
- The version is greater than the newest tag: `git tag --sort=-v:refname | head -3`.
- A breaking change means a new major, a feature a new minor, a fix a new patch.

## 2. The tree is green

```sh
cargo fmt --all --check
cargo build --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked -- --test-threads=4
sh tests/license/run.sh && sh tests/release/run.sh && sh tests/install/run.sh
```

## 3. The release commit

- Set `version` under `[workspace.package]` in `Cargo.toml` to the new version, then
  `cargo build --workspace --locked` so `Cargo.lock` follows.
- Write the `CHANGELOG.md` section: `## [<version>] - <today>` with `### Added`, `### Changed`,
  `### Fixed` or `### Removed` under it, and the compare link at the bottom of the file. Read
  `git log --oneline <last tag>..HEAD` and write what a reader of the release notes needs, not a
  list of commits.
- `sh scripts/release/check-version.sh v<version>` and
  `sh scripts/release/changelog-section.sh <version>` both answer.
- Commit: `Release <version>`.

## 4. Ask the author

Print the version, the changelog section and the commit, and ask before the tag. A tag that is
pushed is public; nothing after this point is undone quietly.

## 5. Tag and push

```sh
git push
git tag v<version>
git push origin v<version>
```

## 6. Watch the pipeline

`gh run watch --exit-status`. It builds four archives, smoke-tests each on a runner of its own
platform, verifies `SHA256SUMS` and publishes the release. A failure before `publish` means
nothing was published: fix it, delete the tag on both sides, and tag again.

## 7. Check the release and the installer

```sh
gh release view v<version> --json assets --jq '[.assets[].name]'
```

Four archives and `SHA256SUMS`. Then install it the way a reader would, into a throwaway
directory so the check does not touch the real one:

```sh
DOMUX_INSTALL_DIR=$(mktemp -d) DOMUX_HOOKS=no DOMUX_STAY_AWAKE=no \
  sh -c 'curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh'
```

It ends with the installed path and the version on stdout.
