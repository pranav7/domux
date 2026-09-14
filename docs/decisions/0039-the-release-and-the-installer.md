# 0039: The release and the installer

Date: 2026-09-11
Status: accepted; amended by 0043, which adds the leader and stay awake questions and brings the
spinner back for the steps that wait on the network, and by 0047, which serves `install.sh` from
domux.dev

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
what to do next. It now also installs the agent hooks, asks for the leader, and asks whether to
set up stay awake for a closed lid, because the hooks are the difference between an agent row
that says what an agent is doing and one that says unknown, and a reader who has to run three
more commands will run one of them. It installs hooks only for an agent whose configuration
directory is already there, so it never creates configuration for a tool the reader does not
use, and the questions are asked only when there is a terminal to ask on. `DOMUX_HOOKS=no`,
`DOMUX_LEADER=<key>` and `DOMUX_STAY_AWAKE=yes|no` answer all three without a prompt, which is
also how the tests drive those branches. Decision 0043 records the two questions and what their
answers write.

**Only a step that waits on the network moves.** The first version put the band along the logo
and along the word of a running step, the way domux draws an agent row. Both were invisible in
use: the logo sweep is over in under a second, and a reader watching an install cannot follow a
wave travelling through a word they are also trying to read. The braille spinner that replaced
them turned beside every step and read as a flicker, so for a while nothing animated.

That reasoning measured every step as finishing in a fraction of a second, which is true of the
steps that run on the machine and of a download on a fast connection. It is not true of the
release lookup and the download on a real connection, where a 3 MB archive takes long enough to
read a spinner on, and a word that stands still leaves the reader unable to tell a slow network
from a script that has stopped. So the spinner is back on those two steps and on nothing else.
A frame or two that turns into a check mark before it can be read is the flicker the earlier
spinner made, so the first frame waits a quarter second and a request that answers sooner draws
nothing. A request that answers just after the quarter second still draws a single frame, which
the check mark replaces at once; decision 0043 says why that edge is left. Every other step
prints its check mark as soon as it is done.

The logo is printed once. A finished step is a check mark, a step that did not work while the
install carries on is a cross, and a question wears the red dot, which means the same thing there
as it does on an agent row. The color is still domux's mauve. Without a terminal, and under
`NO_COLOR`, nothing moves and the same lines are printed in the same order, so a log file reads
as well as a terminal does.

A curl installer cannot take a dependency for any of this. It has to be one POSIX sh file that
runs on a machine with nothing on it, so the spinner is a background job and a list of frames in
the script, and the frames are the braille set every command line tool uses rather than
something invented here. A trap stops the request, clears the line and gives the cursor back
when the script exits, fails, or is interrupted.

**The line a script reads is only printed when nothing is watching.** The installed path and
version go to stdout for a caller that pipes the script, and stdout under `curl | sh` is the
reader's terminal, where an unstyled path in the middle of the steps is noise. So that line is
written only when stdout is not a terminal; a reader gets the step that says the same thing.

**The archive is checked before it is trusted.** The installer downloads the archive and
`SHA256SUMS`, compares, and refuses to install on a mismatch. Three network requests, all to
GitHub. No shell startup file is edited: the PATH line is printed for the reader to paste. The
config file gets the lines decision 0043 describes and nothing else.

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
`tests/install/` fakes curl, uname, ldd and sudo so every branch of the installer runs without a
network and nothing it runs can raise privileges, and CI runs all of it under dash, bash and
macOS's sh, with shellcheck over every script. A failure names the state and the next action, the
same rule the Rust errors follow.

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
