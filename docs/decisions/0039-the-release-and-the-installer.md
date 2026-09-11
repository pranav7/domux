# 0039: The release and the installer

Date: 2026-09-11
Status: accepted

## Context

M5 puts domux in someone else's hands. That needs release builds, a way to get one, and a
license that says what the binary may be used for. The specs leave the shape of all three open,
and the M5 plan wrote a longer version of it than the first release needs.

## Decisions

**The public version is 1.0.0.** The names V2 and domux2 are internal, for the rewrite
generation. The public never had a 1.x, so the first release of the Rust version is 1.0.0 and
the Go version keeps the v0.x tags it already published. `scripts/release/check-version.sh`
refuses a v0 tag for that reason, and the installer skips v0 releases when it looks for the
newest one.

**Apache-2.0, with attribution.** V2 is Apache-2.0 for the patent grant that MIT does not
carry. V1 stays MIT on its own tags. Every archive carries `LICENSE`, `NOTICE` for the Ghostty
library compiled in, and `THIRD_PARTY_LICENSES.md`, generated at release time by cargo-about
from `about.toml`. `accepted` in that file is the list of licenses a compiled crate may carry,
and generation fails on anything else, so a new dependency with a copyleft license stops a
release rather than shipping quietly.

**One install path: the curl script.** No package manager, no update command. Re-running the
command upgrades. `install.sh` lives at the repository root and is served from the raw URL on
`main`.

**The installer does the setup, not only the copy.** The plan had it copy the binary and print
what to do next. It now also installs the agent hooks and offers the macOS lid setup, because
the hooks are the difference between an agent row that says what an agent is doing and one that
says unknown, and a reader who has to run three more commands will run one of them. It installs
hooks only for an agent whose configuration directory is already there, so it never creates
configuration for a tool the reader does not use, and the lid question is asked only when there
is a terminal to ask on. `DOMUX_HOOKS=no` and `DOMUX_STAY_AWAKE=yes|no` answer both without a
prompt, which is also how the tests drive those branches.

**The archive is checked before it is trusted.** The installer downloads the archive and
`SHA256SUMS`, compares, and refuses to install on a mismatch. Three network requests, all to
GitHub. Nothing else is written and no shell startup file is edited: the PATH line is printed
for the reader to paste.

**Four targets, each smoke-tested on its own platform.** macOS on Apple silicon and Intel, Linux
on x86_64 and arm64. Linux archives build on the oldest hosted image so the glibc floor is 2.35;
they are smoke-tested on a newer one. macOS binaries are ad-hoc signed in the pipeline, because
Apple silicon refuses to run arm64 code with no signature at all and stripping can invalidate
the signature the linker added. That is not a Developer ID signature and there is no
notarization: a browser download needs one `xattr -d` command, which the README gives, and the
curl path needs nothing.

**The smoke test runs `api schema`, not `doctor`.** `doctor` was M4's and M4 was closed before
it was built. `api schema` answers from the build itself rather than from a running server, so
it proves the binary loads and its control API is intact without starting anything. The test
also fails if a socket appears.

**Shell code is tested like the rest.** `tests/lib/assert.sh` is the assertion harness,
`tests/install/` fakes curl, uname and ldd so every branch of the installer runs without a
network, and CI runs all of it under dash, bash and macOS's sh, with shellcheck over every
script. A failure names the state and the next action, the same rule the Rust errors follow.

## Consequences

- A release is one tag: `git tag v1.0.0 && git push origin v1.0.0` builds, smoke-tests and
  publishes. The tag must equal the version in `Cargo.toml` and have a `CHANGELOG.md` section,
  or the pipeline stops before it builds anything.
- `cargo install cargo-about` needs `--features cli`; without it the library compiles and no
  binary is installed.
- A dependency whose license is not in `about.toml` fails CI, and adding one to that list is a
  deliberate commit.
- The installer's version lookup uses the unauthenticated GitHub API, which allows 60 requests
  an hour per address. `DOMUX_VERSION` skips the lookup, and the failure message says so.
