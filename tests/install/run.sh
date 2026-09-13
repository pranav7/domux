#!/bin/sh
# Tests for install.sh, one per branch of the script. A fake curl, uname, ldd and sudo sit first
# on PATH; tar, mktemp and sha256sum or shasum are the real ones. Every test builds its own fake
# release under a sandbox and installs into a sandbox directory; nothing touches the real HOME.
# Run: sh tests/install/run.sh
# TEST_SHELL=bash sh tests/install/run.sh runs install.sh under bash instead of sh.
#
# The tests that call on_a_terminal run the installer on a terminal through util-linux script.
# BSD script takes other flags, so those tests skip on macOS, and CI runs them on Linux.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=tests/lib/assert.sh
. tests/lib/assert.sh
ROOT=$(pwd)
FAKEBIN="$ROOT/tests/install/fakebin"
TEST_SHELL=${TEST_SHELL:-sh}
SANDBOX_ROOT=$(mktemp -d)
trap 'rm -rf "$SANDBOX_ROOT"' EXIT INT TERM
ZEROS=0000000000000000000000000000000000000000000000000000000000000000
ESC=$(printf '\033')
CR=$(printf '\r')
HIDE="${ESC}[?25l"
SHOW="${ESC}[?25h"
EOL="${ESC}[K"
FRAMES="⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏"
# A job a script starts in the background begins with SIGINT ignored, and a shell cannot trap a
# signal it started out ignoring. GNU env gives the installer SIGINT back the way a terminal
# would hand it over; without it the test that sends SIGINT skips.
if env --default-signal=INT true 2>/dev/null; then
  DEFAULT_INT="--default-signal=INT"
else
  DEFAULT_INT=""
fi

# sandbox: a fresh home, install dir, temp dir and fake http root for one test; macOS arm64 by default.
sandbox() {
  S="$SANDBOX_ROOT/$CURRENT"
  mkdir -p "$S/home" "$S/bin" "$S/tmp" "$S/http"
  FAKE_HTTP_DIR="$S/http"
  : > "$FAKE_HTTP_DIR/releases.json"
  : > "$FAKE_HTTP_DIR/requests.log"
  FAKE_CURL_FAIL=""
  FAKE_CURL_DELAY=""
  FAKE_DOMUX_DELAY=""
  FAKE_LDD=""
  FAKE_UNAME_S=Darwin
  FAKE_UNAME_M=arm64
  CONFIG="$S/home/.config/domux/domux.toml"
}

# releases <tag>...: the fake GitHub API answer, in the order given (the API lists newest first).
releases() {
  {
    printf '['
    sep=""
    for rel_tag in "$@"; do
      printf '%s{"tag_name": "%s", "draft": false}' "$sep" "$rel_tag"
      sep=","
    done
    printf ']\n'
  } > "$FAKE_HTTP_DIR/releases.json"
}

# release <tag> <os> <arch> [mode] [subcommand exit code]: a fake release archive plus its
# SHA256SUMS line. The packed binary answers --version and logs every call to $S/calls.log, so a
# test can say which setup commands the installer ran. Every other call takes FAKE_DOMUX_DELAY
# seconds when that is set.
# mode: ok (default) | broken (the binary exits 126) | empty (no binary inside) | badsum
release() {
  tag=$1; os=$2; arch=$3; mode=${4:-ok}; sub_code=${5:-0}
  body=${tag#v}
  dir="$FAKE_HTTP_DIR/$tag"
  stage="$S/stage-$tag-$os-$arch"
  mkdir -p "$dir" "$stage"
  printf 'license\n' > "$stage/LICENSE"
  case $mode in
    empty) ;;
    broken) printf '#!/bin/sh\necho "cannot execute binary file" >&2\nexit 126\n' > "$stage/domux"; chmod 0755 "$stage/domux" ;;
    *)
      cat > "$stage/domux" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$S/calls.log"
if [ "\${1:-}" = --version ]; then printf 'domux $body\n'; exit 0; fi
[ -z "\${FAKE_DOMUX_DELAY:-}" ] || sleep "\$FAKE_DOMUX_DELAY"
printf 'fake domux ran: %s\n' "\$*"
exit $sub_code
EOF
      chmod 0755 "$stage/domux" ;;
  esac
  archive="domux_${body}_${os}_${arch}.tar.gz"
  (cd "$stage" && COPYFILE_DISABLE=1 tar -czf "$dir/$archive" -- *)
  if command -v sha256sum >/dev/null 2>&1; then
    sum=$(sha256sum "$dir/$archive" | cut -d' ' -f1)
  else
    sum=$(shasum -a 256 "$dir/$archive" | cut -d' ' -f1)
  fi
  [ "$mode" = badsum ] && sum=$ZEROS
  printf '%s  %s\n' "$sum" "$archive" >> "$dir/SHA256SUMS"
}

# run_install [VAR=value ...]: runs install.sh with the fakes first on PATH, HOME and TMPDIR in the
# sandbox, and DOMUX_INSTALL_DIR=$S/bin unless a later VAR=value overrides it. Sets code, $S/out, $S/err.
run_install() {
  env -i HOME="$S/home" TMPDIR="$S/tmp" PATH="$FAKEBIN:$PATH" \
    FAKE_HTTP_DIR="$FAKE_HTTP_DIR" FAKE_CURL_FAIL="$FAKE_CURL_FAIL" FAKE_LDD="$FAKE_LDD" \
    FAKE_CURL_DELAY="$FAKE_CURL_DELAY" FAKE_DOMUX_DELAY="$FAKE_DOMUX_DELAY" \
    FAKE_UNAME_S="$FAKE_UNAME_S" FAKE_UNAME_M="$FAKE_UNAME_M" \
    DOMUX_INSTALL_DIR="$S/bin" "$@" "$TEST_SHELL" "$ROOT/install.sh" >"$S/out" 2>"$S/err"
  code=$?
}

# on_a_terminal: true when util-linux script is there to give the installer a terminal; otherwise
# says the test is skipped.
on_a_terminal() {
  if script --version 2>/dev/null | grep -q util-linux; then
    return 0
  fi
  printf 'skip %s: needs util-linux script to run on a terminal\n' "$CURRENT" >&2
  return 1
}

# start_install_tty [VAR=value ...]: run_install on a terminal, in the background. What the
# terminal shows, stdout and stderr together, lands in $S/tty; what the reader types comes from
# $S/keys, a file or a pipe the test makes first. Sets TTY_PID.
start_install_tty() {
  env ${DEFAULT_INT:+"$DEFAULT_INT"} -i HOME="$S/home" TMPDIR="$S/tmp" PATH="$FAKEBIN:$PATH" \
    FAKE_HTTP_DIR="$FAKE_HTTP_DIR" FAKE_CURL_FAIL="$FAKE_CURL_FAIL" FAKE_LDD="$FAKE_LDD" \
    FAKE_CURL_DELAY="$FAKE_CURL_DELAY" FAKE_DOMUX_DELAY="$FAKE_DOMUX_DELAY" \
    FAKE_UNAME_S="$FAKE_UNAME_S" FAKE_UNAME_M="$FAKE_UNAME_M" \
    DOMUX_INSTALL_DIR="$S/bin" "$@" \
    script -qec "\"$TEST_SHELL\" \"$ROOT/install.sh\"" /dev/null <"$S/keys" >"$S/tty" 2>&1 &
  TTY_PID=$!
}

# run_install_tty [VAR=value ...]: start_install_tty with nothing typed, then waits. Sets code.
run_install_tty() {
  : > "$S/keys"
  start_install_tty "$@"
  wait "$TTY_PID"
  code=$?
}

# wait_for <file> <text>: true once the file holds the text; false after about ten seconds.
wait_for() {
  wf_i=0
  while [ "$wf_i" -lt 100 ]; do
    case $(cat "$1" 2>/dev/null) in *"$2"*) return 0 ;; esac
    sleep 0.1
    wf_i=$((wf_i + 1))
  done
  return 1
}

# type_after <text> <keys>: once the terminal shows the text, types the keys on descriptor 3.
# The keys go through printf's %b, so '\023' is C-s and '\0' is C-Space. The write happens in a
# subshell that ignores SIGPIPE, so keys typed after the installer has exited fail the write
# rather than killing this runner.
type_after() {
  if wait_for "$S/tty" "$1"; then
    sleep 0.3
    (trap '' PIPE; printf '%b' "$2" >&3) 2>/dev/null
  else
    fail "the terminal never showed '$1': $(tty_text)"
    return 1
  fi
}

err() { cat "$S/err"; }
out() { cat "$S/out"; }
tty_text() { cat "$S/tty"; }
requests() { cat "$FAKE_HTTP_DIR/requests.log"; }
calls() { cat "$S/calls.log" 2>/dev/null; }
config() { cat "$CONFIG" 2>/dev/null; }

# has_frame <text>: true when the text holds any spinner frame.
has_frame() {
  for hf_frame in $FRAMES; do
    case $1 in *"$hf_frame"*) return 0 ;; esac
  done
  return 1
}

# mode_of <path>: the permission bits as ls prints them, for example -rw-------. stat takes other
# flags on macOS than on Linux, and ls prints the bits the same way on both.
mode_of() {
  # shellcheck disable=SC2012  # one path the test made, and only the bits are read
  ls -l "$1" | cut -c1-10
}

test_installs_the_newest_stable_release_on_macos_arm64() {
  sandbox
  releases v1.1.0-beta.1 v1.0.5 v1.0.4 v0.3.0
  release v1.0.5 darwin arm64
  release v1.1.0-beta.1 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$S/bin/domux 1.0.5" "$(out)" "stdout data line"
  assert_file "$S/bin/domux" "installed binary"
  assert_eq "domux 1.0.5" "$("$S/bin/domux" --version)" "installed binary runs"
  assert_contains "$(err)" "domux 1.0.5 installed to $S/bin/domux" "summary"
  assert_contains "$(err)" "release v1.0.5 found" "release line"
  assert_contains "$(err)" "domux is ready" "ready line"
  assert_contains "$(err)" "> run domux" "what to run"
  assert_contains "$(err)" "domux install claude --apply" "next steps"
  assert_contains "$(err)" "macos on arm64 detected" "platform line"
  assert_contains "$(requests)" "releases/download/v1.0.5/domux_1.0.5_darwin_arm64.tar.gz" "archive request"
  assert_contains "$(requests)" "releases/download/v1.0.5/SHA256SUMS" "checksums request"
  assert_eq 3 "$(wc -l < "$FAKE_HTTP_DIR/requests.log" | tr -d ' ')" "exactly three requests"
  assert_eq "" "$(ls "$S/tmp")" "temp dir removed"
}

test_prints_the_logo_before_anything_else() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_eq "" "$(sed -n 1p "$S/err")" "first line is blank"
  assert_contains "$(sed -n 2p "$S/err")" "█▀▄ █▀█ █▀▄▀█ █ █ ▀▄▀" "logo line 1"
  assert_contains "$(sed -n 3p "$S/err")" "█▄▀ █▄█ █ ▀ █ █▄█ █ █" "logo line 2"
  assert_contains "$(sed -n 3p "$S/err")" "github.com/pranav7/domux" "repository under the logo"
}

test_marks_every_finished_step_with_a_tick() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  mkdir -p "$S/home/.claude"
  releases v1.0.0
  release v1.0.0 linux amd64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(err)" "   ✓  linux on amd64 detected" "platform"
  assert_contains "$(err)" "   ✓  release v1.0.0 found" "release"
  assert_contains "$(err)" " downloaded, checksum verified" "download"
  assert_contains "$(err)" "   ✓  domux 1.0.0 installed to $S/bin/domux" "install"
  assert_contains "$(err)" "   ✓  claude detected, hooks installed" "hooks"
  assert_contains "$(err)" "   ✓  leader C-a written to $CONFIG" "leader"
  assert_contains "$(err)" "   ✓  stay awake set to full in $CONFIG" "stay awake"
  assert_contains "$(err)" "   ✓  domux is ready" "ready"
  assert_eq 0 "$(grep -c '^   ·' "$S/err")" "no step keeps the faint dot"
  assert_eq 0 "$(grep -v '^   ✓  ' "$S/err" | grep -c -E 'detected|found|downloaded|installed to|written|set to|ready')" "every step line starts with the tick"
}

test_falls_back_to_a_prerelease_when_no_stable_release_exists() {
  sandbox
  releases v1.0.0-alpha.2 v1.0.0-alpha.1 v0.3.0
  release v1.0.0-alpha.2 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$S/bin/domux 1.0.0-alpha.2" "$(out)" "stdout"
}

test_installs_a_pinned_version_on_linux_amd64() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  release v1.0.1 linux amd64
  run_install DOMUX_VERSION=v1.0.1
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$S/bin/domux 1.0.1" "$(out)" "stdout"
  assert_not_contains "$(requests)" "api.github.com" "no release lookup when pinned"
  assert_contains "$(err)" "✓  release v1.0.1 pinned by DOMUX_VERSION" "release line"
}

test_maps_aarch64_to_arm64_and_accepts_a_version_without_v() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=aarch64
  release v1.0.1 linux arm64
  run_install DOMUX_VERSION=1.0.1
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(requests)" "domux_1.0.1_linux_arm64.tar.gz" "aarch64 maps to arm64"
}

test_maps_amd64_to_amd64_on_macos() {
  sandbox
  FAKE_UNAME_M=x86_64
  release v1.0.1 darwin amd64
  run_install DOMUX_VERSION=v1.0.1
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(requests)" "domux_1.0.1_darwin_amd64.tar.gz" "x86_64 maps to amd64"
}

test_installs_into_domux_install_dir() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_INSTALL_DIR="$S/elsewhere/bin"
  assert_exit 0 "$code" "exit: $(err)"
  assert_file "$S/elsewhere/bin/domux" "binary in DOMUX_INSTALL_DIR"
  assert_eq "$S/elsewhere/bin/domux 1.0.0" "$(out)" "stdout"
}

test_prints_the_path_line_when_the_install_dir_is_not_on_path() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_contains "$(err)" "$S/bin is not on your PATH" "state"
  assert_contains "$(err)" "export PATH=\"$S/bin:\$PATH\"" "the line to add"
  assert_contains "$(err)" "fish_add_path $S/bin" "fish alternative"
  assert_not_contains "$(out)" "PATH" "hint is not on stdout"
}

test_prints_no_path_line_when_the_install_dir_is_on_path() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install PATH="$S/bin:$FAKEBIN:$PATH"
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(err)" "not on your PATH" "no hint"
}

test_never_edits_shell_startup_files() {
  sandbox
  printf '# untouched\n' > "$S/home/.zshrc"
  printf '# untouched\n' > "$S/home/.bashrc"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_eq "# untouched" "$(cat "$S/home/.zshrc")" ".zshrc"
  assert_eq "# untouched" "$(cat "$S/home/.bashrc")" ".bashrc"
  assert_eq "./.bashrc
./.config
./.config/domux
./.config/domux/domux.toml
./.zshrc" "$(cd "$S/home" && find . -mindepth 1 | LC_ALL=C sort)" "nothing under HOME but the config file"
}

test_replaces_an_existing_binary() {
  sandbox
  printf '#!/bin/sh\nprintf "domux 0.9.9\\n"\n' > "$S/bin/domux"
  chmod 0755 "$S/bin/domux"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "domux 1.0.0" "$("$S/bin/domux" --version)" "new binary"
  assert_no_file "$S/bin/domux.tmp" "no temp file left"
}

test_fails_on_an_unsupported_os() {
  sandbox
  FAKE_UNAME_S=FreeBSD; FAKE_UNAME_M=amd64
  run_install
  assert_exit 1 "$code" "exit"
  assert_eq "" "$(out)" "stdout"
  assert_contains "$(err)" "domux install: domux runs on macOS and Linux; this system is FreeBSD" "state"
  assert_contains "$(err)" "  next: build from source" "next action"
  assert_eq "" "$(requests)" "no network request"
}

test_fails_on_an_unsupported_arch() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=i686
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "no release build for the i686 architecture" "state"
}

test_fails_on_musl() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64; FAKE_LDD="musl libc (x86_64)"
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "release builds link glibc; this system uses musl" "state"
}

test_fails_when_curl_is_missing() {
  sandbox
  mkdir "$S/nocurl"
  for tool in sh bash tar mktemp grep sed cut tr head mkdir chmod mv rm cat ls id sha256sum shasum; do
    p=$(command -v "$tool" 2>/dev/null) && ln -s "$p" "$S/nocurl/$tool"
  done
  ln -s "$FAKEBIN/uname" "$S/nocurl/uname"
  run_install PATH="$S/nocurl"
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "domux install: curl is not installed" "state"
  assert_contains "$(err)" "  next: install curl" "next action"
}

test_fails_when_the_release_list_cannot_be_fetched() {
  sandbox
  FAKE_CURL_FAIL="api.github.com"
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "curl: (22) The requested URL returned error: 404" "what curl said"
  assert_contains "$(err)" "domux install: could not list releases at https://api.github.com/repos/pranav7/domux/releases" "state"
  assert_contains "$(err)" "set DOMUX_VERSION=v1.x.y to skip the lookup" "next action"
}

test_fails_when_no_release_exists() {
  sandbox
  releases v0.3.0 v0.2.0
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "domux install: no domux release exists yet" "state"
}

test_fails_when_domux_version_is_a_v1_go_tag() {
  sandbox
  run_install DOMUX_VERSION=v0.3.0
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "release v0.3.0 is domux V1, the Go version" "state"
}

test_fails_when_domux_version_is_not_a_tag() {
  sandbox
  run_install DOMUX_VERSION=latest
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" 'DOMUX_VERSION must be a release tag such as v1.0.0; got "latest"' "state"
}

test_fails_before_any_request_when_domux_leader_is_not_a_key_name() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=ctrl-s
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" 'DOMUX_LEADER must be a key name such as C-s, M-a or C-Space; got "ctrl-s"' "state"
  assert_contains "$(err)" "  next: set DOMUX_LEADER to a key name" "next action"
  assert_eq "" "$(requests)" "no network request"
  assert_no_file "$S/bin/domux" "nothing installed"
}

test_fails_when_the_archive_is_missing_from_the_release() {
  sandbox
  releases v1.0.0
  release v1.0.0 linux amd64
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "could not download https://github.com/pranav7/domux/releases/download/v1.0.0/domux_1.0.0_darwin_arm64.tar.gz" "state"
  assert_no_file "$S/bin/domux" "nothing installed"
}

test_fails_when_checksums_are_missing() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  rm "$FAKE_HTTP_DIR/v1.0.0/SHA256SUMS"
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "could not download https://github.com/pranav7/domux/releases/download/v1.0.0/SHA256SUMS" "state"
  assert_no_file "$S/bin/domux" "nothing installed"
}

test_fails_when_checksums_have_no_entry_for_the_archive() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  release v1.0.0 linux amd64
  grep -v darwin_arm64 "$FAKE_HTTP_DIR/v1.0.0/SHA256SUMS" > "$S/sums"
  mv "$S/sums" "$FAKE_HTTP_DIR/v1.0.0/SHA256SUMS"
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "SHA256SUMS for v1.0.0 has no entry for domux_1.0.0_darwin_arm64.tar.gz" "state"
}

test_fails_when_the_checksum_mismatches() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64 badsum
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "checksum mismatch for domux_1.0.0_darwin_arm64.tar.gz" "state"
  assert_contains "$(err)" "do not run the downloaded file" "next action"
  assert_no_file "$S/bin/domux" "nothing installed"
  assert_eq "" "$(ls "$S/tmp")" "download removed"
}

test_fails_when_the_archive_has_no_binary() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64 empty
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "domux_1.0.0_darwin_arm64.tar.gz has no domux binary inside" "state"
}

test_fails_when_the_install_dir_is_not_writable() {
  sandbox
  if [ "$(id -u)" = 0 ]; then
    printf 'skip %s: root can write anywhere\n' "$CURRENT" >&2
    return
  fi
  mkdir "$S/ro" && chmod 0555 "$S/ro"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_INSTALL_DIR="$S/ro"
  chmod 0755 "$S/ro"
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "cannot write to $S/ro" "state"
  assert_contains "$(err)" "set DOMUX_INSTALL_DIR" "next action"
}

test_fails_when_the_installed_binary_does_not_run() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64 broken
  run_install
  assert_exit 1 "$code" "exit"
  assert_contains "$(err)" "cannot execute binary file" "the binary's own message is shown"
  assert_contains "$(err)" "$S/bin/domux was installed but does not run on this system" "state"
  assert_contains "$(err)" "glibc 2.35" "next action names the glibc floor"
}

test_says_each_agent_it_detected_and_installs_its_hooks() {
  sandbox
  mkdir -p "$S/home/.claude" "$S/home/.config/opencode"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(calls)" "install claude --apply" "claude hooks"
  assert_contains "$(calls)" "install opencode --apply" "opencode hooks"
  assert_not_contains "$(calls)" "install codex --apply" "codex is not configured here"
  assert_contains "$(err)" "✓  claude detected, hooks installed" "a line for claude"
  assert_contains "$(err)" "✓  opencode detected, hooks installed" "a line for opencode"
  assert_not_contains "$(err)" "codex" "no line for an agent that is not there"
}

test_says_no_agent_was_detected_when_none_is_configured() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "install " "no hooks installed"
  assert_contains "$(err)" "✓  no coding agent detected, so no hooks were installed" "state"
  assert_contains "$(err)" "install claude --apply once Claude Code, Codex or OpenCode is there" "next action"
}

test_follows_claude_config_dir_for_the_claude_hooks() {
  sandbox
  mkdir -p "$S/elsewhere/claude"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install CLAUDE_CONFIG_DIR="$S/elsewhere/claude"
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(calls)" "install claude --apply" "claude hooks follow the variable"
}

test_says_what_happened_when_a_hook_install_fails() {
  sandbox
  mkdir -p "$S/home/.claude"
  releases v1.0.0
  release v1.0.0 darwin arm64 ok 1
  run_install
  assert_exit 0 "$code" "exit: the installer still installed the binary"
  assert_contains "$(err)" "✗  claude detected, hooks not installed: fake domux ran: install claude --apply" "state"
  assert_contains "$(err)" "install claude --apply to see what happened" "next action"
  assert_not_contains "$(err)" "✓  claude detected" "no tick on a step that failed"
}

test_skips_the_hooks_when_asked() {
  sandbox
  mkdir -p "$S/home/.claude"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_HOOKS=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "--apply" "nothing applied"
  assert_contains "$(err)" "✓  hooks skipped" "state"
}

test_writes_the_leader_into_a_config_file_that_does_not_exist_yet() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[keys]
leader = "C-a"' "$(config)" "the config file"
  assert_contains "$(err)" "✓  leader C-a written to $CONFIG" "state"
  assert_no_file "$CONFIG.tmp" "no temp file left"
}

test_adds_the_leader_to_a_config_file_with_other_tables() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '# mine\n[terminal]\nscrollback = 5000\n\n[keys.bindings]\nx = "pane.zoom"\n' > "$CONFIG"
  chmod 0600 "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-b DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '# mine
[terminal]
scrollback = 5000

[keys.bindings]
x = "pane.zoom"

[keys]
leader = "C-b"' "$(config)" "every line kept and the table added"
  assert_eq "-rw-------" "$(mode_of "$CONFIG")" "the file keeps its permissions"
}

test_adds_the_leader_under_keys_when_keys_has_no_leader() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '[terminal]\nshell = "zsh"\n\n[keys]  # my keys\n\n[keys.global]\n"C-g" = "pane.zoom"\n\n[navigator]\nenabled = false\n' > "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=M-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[terminal]
shell = "zsh"

[keys]  # my keys
leader = "M-a"

[keys.global]
"C-g" = "pane.zoom"

[navigator]
enabled = false' "$(config)" "the leader goes right under [keys]"
}

test_leaves_a_leader_that_is_already_set_and_says_which() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '[keys]\nleader = "C-b" # tmux hands\n' > "$CONFIG"
  cp "$CONFIG" "$S/before"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$(cat "$S/before")" "$(config)" "the config file is unchanged"
  assert_contains "$(err)" "✓  leader C-b already set in $CONFIG" "names the leader that is set"
  assert_contains "$(err)" "DOMUX_LEADER does not replace a leader the config file sets" "says why the variable did nothing"
}

test_writes_to_domux_config_file() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_CONFIG_FILE="$S/elsewhere/domux.toml" DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[keys]
leader = "C-a"' "$(cat "$S/elsewhere/domux.toml" 2>/dev/null)" "the file DOMUX_CONFIG_FILE names"
  assert_no_file "$S/home/.config" "nothing at the default path"
  assert_contains "$(err)" "leader C-a written to $S/elsewhere/domux.toml" "state"
}

test_writes_the_default_leader_when_there_is_no_terminal_to_ask_on() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[keys]
leader = "C-s"' "$(config)" "the default"
  assert_contains "$(err)" "✓  leader C-s written to $CONFIG" "state"
  assert_contains "$(err)" "the default, since there is no terminal to ask on" "says why"
  assert_not_contains "$(err)" "Select your leader" "asks nothing"
}

test_escapes_a_leader_that_toml_would_misread() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install "DOMUX_LEADER=C-\\" DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[keys]
leader = "C-\\"' "$(config)" "a backslash is escaped"
}

test_writes_through_a_linked_config_file_and_keeps_the_link() {
  sandbox
  mkdir -p "$S/home/.config/domux" "$S/dotfiles"
  printf '[terminal]\nscrollback = 100\n' > "$S/dotfiles/domux.toml"
  ln -s "$S/dotfiles/domux.toml" "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  if [ -L "$CONFIG" ]; then :; else fail "the link was replaced by a file"; fi
  assert_eq '[terminal]
scrollback = 100

[keys]
leader = "C-a"' "$(cat "$S/dotfiles/domux.toml")" "the file the link points at"
  assert_no_file "$S/dotfiles/domux.toml.tmp" "no temp file left"
}

test_leaves_a_config_file_that_sets_keys_without_a_table_alone() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf 'keys = { passthrough = { commands = ["vim"] } }\n' > "$CONFIG"
  cp "$CONFIG" "$S/before"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: the binary is still installed"
  assert_eq "$(cat "$S/before")" "$(config)" "the config file is unchanged"
  assert_contains "$(err)" "✗  no leader written: $CONFIG sets keys outside a [keys] table" "state"
  assert_contains "$(err)" 'add leader = "C-a" to keys in it by hand' "next action"
}

test_reads_a_header_inside_a_multi_line_array_or_string_as_part_of_it() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '[terminal]\nshell = """\n[keys]\nleader = "C-b"\n"""\n\n[x]\narr = [\n  ["keys"]\n]\n' > "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[terminal]
shell = """
[keys]
leader = "C-b"
"""

[x]
arr = [
  ["keys"]
]

[keys]
leader = "C-a"' "$(config)" "the string and the array are left whole, and the table is added after them"
}

test_adds_the_leader_under_a_keys_header_written_with_quotes() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf "['keys']\n\n[ \"stay_awake\" ]\nmode = \"full\"\n" > "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "['keys']
leader = \"C-a\"

[ \"stay_awake\" ]
mode = \"full\"" "$(config)" "the leader under the one keys table"
  assert_contains "$(err)" "✓  stay awake already set to full in $CONFIG" "the quoted stay_awake header"
}

test_adds_the_leader_under_a_header_after_a_byte_order_mark() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '\357\273\277[keys]\n\n[terminal]\nscrollback = 10\n' > "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$(printf '\357\273\277[keys]\nleader = "C-a"\n\n[terminal]\nscrollback = 10')" "$(config)" "the leader under the one keys table"
}

test_reads_a_leader_under_a_quoted_key_as_set() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  releases v1.0.0
  release v1.0.0 darwin arm64
  printf '[keys]\n"leader" = "C-b"\n' > "$CONFIG"
  cp "$CONFIG" "$S/before"
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$(cat "$S/before")" "$(config)" "a quoted key: the config file is unchanged"
  assert_contains "$(err)" "✓  leader C-b already set in $CONFIG" "a quoted key"
  printf "\"keys\".'leader' = 'M-b'\n" > "$CONFIG"
  cp "$CONFIG" "$S/before"
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$(cat "$S/before")" "$(config)" "a quoted dotted key: the config file is unchanged"
  assert_contains "$(err)" "✓  leader M-b already set in $CONFIG" "a quoted dotted key"
}

test_reads_a_leader_in_a_multi_line_string_as_set() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  releases v1.0.0
  release v1.0.0 darwin arm64
  printf '[keys]\nleader = """C-b"""\n' > "$CONFIG"
  cp "$CONFIG" "$S/before"
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$(cat "$S/before")" "$(config)" "on one line: the config file is unchanged"
  assert_contains "$(err)" "✓  leader C-b already set in $CONFIG" "on one line"
  printf "[keys]\nleader = '''\nM-b'''\n" > "$CONFIG"
  cp "$CONFIG" "$S/before"
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "$(cat "$S/before")" "$(config)" "over two lines: the config file is unchanged"
  assert_contains "$(err)" "✓  leader M-b already set in $CONFIG" "over two lines"
}

test_reads_a_leader_that_is_a_question_mark_or_an_exclamation_mark_as_set() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  releases v1.0.0
  release v1.0.0 darwin arm64
  for mark in '?' '!'; do
    printf '[keys]\nleader = "%s"\n\n[stay_awake]\nmode = "%s"\n' "$mark" "$mark" > "$CONFIG"
    run_install DOMUX_STAY_AWAKE=no
    assert_exit 0 "$code" "exit: $(err)"
    assert_contains "$(err)" "✓  leader $mark already set in $CONFIG" "the leader $mark"
    assert_contains "$(err)" "✓  stay awake already set to $mark in $CONFIG" "the mode $mark"
    assert_not_contains "$(err)" "✗" "no step failed"
  done
}

test_leaves_a_read_only_config_file_alone_without_a_shell_error() {
  sandbox
  if [ "$(id -u)" = 0 ]; then
    printf 'skip %s: root can write anywhere\n' "$CURRENT" >&2
    return
  fi
  mkdir -p "$S/home/.config/domux"
  printf '[terminal]\nscrollback = 1\n' > "$CONFIG"
  chmod 0444 "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  chmod 0644 "$CONFIG"
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[terminal]
scrollback = 1' "$(config)" "the config file is unchanged"
  assert_contains "$(err)" "✗  could not write the leader to $CONFIG" "state"
  assert_not_contains "$(err)" "Permission denied" "no error from the shell"
  assert_not_contains "$(err)" "cannot create" "no error from the shell"
  assert_no_file "$CONFIG.tmp" "no temp file left"
}

test_replaces_a_leftover_temp_file_link_rather_than_writing_through_it() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '[terminal]\nscrollback = 1\n' > "$CONFIG"
  printf 'keep me\n' > "$S/victim"
  ln -s "$S/victim" "$CONFIG.tmp"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq "keep me" "$(cat "$S/victim")" "the file the link points at is untouched"
  if [ -L "$CONFIG" ]; then fail "the config file became a link"; fi
  assert_eq '[terminal]
scrollback = 1

[keys]
leader = "C-a"' "$(config)" "the config file"
  assert_no_file "$CONFIG.tmp" "no temp file left"
}

test_turns_on_full_stay_awake_on_linux_without_sudo() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  releases v1.0.0
  release v1.0.0 linux amd64
  run_install DOMUX_LEADER=C-b DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_eq '[keys]
leader = "C-b"

[stay_awake]
mode = "full"' "$(config)" "the config file"
  assert_not_contains "$(calls)" "stay-awake" "no setup command on Linux"
  assert_no_file "$FAKE_HTTP_DIR/sudo.log" "no sudo"
  assert_contains "$(err)" "✓  stay awake set to full in $CONFIG" "state"
  assert_contains "$(err)" "press C-b then A inside domux to turn stay awake on or off" "how to turn it on"
}

test_names_the_leader_already_set_for_turning_stay_awake_on() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  mkdir -p "$S/home/.config/domux"
  printf "[keys]\nleader = 'M-Space'\n" > "$CONFIG"
  releases v1.0.0
  release v1.0.0 linux amd64
  run_install DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(err)" "✓  leader M-Space already set in $CONFIG" "a literal string is read too"
  assert_contains "$(err)" "press M-Space then A inside domux" "the leader that is set"
}

test_sets_up_the_lid_and_writes_full_mode_on_macos_when_the_answer_is_yes() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(calls)" "stay-awake install --full --apply" "the command it runs"
  assert_contains "$(err)" "✓  the lid setup is installed" "the setup"
  assert_eq '[keys]
leader = "C-s"

[stay_awake]
mode = "full"' "$(config)" "the line written rather than printed"
  assert_contains "$(err)" "✓  stay awake set to full in $CONFIG" "state"
  assert_contains "$(err)" "press C-s then A inside domux to turn stay awake on or off" "how to turn it on"
}

test_writes_no_stay_awake_mode_when_the_macos_lid_setup_fails() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64 ok 1
  run_install DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(err)" "✗  stay awake could not be set up for a closed lid: fake domux ran: stay-awake install --full --apply" "state"
  assert_not_contains "$(config)" "stay_awake" "no mode the machine cannot hold"
  assert_not_contains "$(err)" "inside domux to turn stay awake" "no key for a setup that failed"
}

test_writes_no_stay_awake_mode_when_the_answer_is_no() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "stay-awake" "nothing run"
  assert_not_contains "$(config)" "stay_awake" "nothing written"
  assert_contains "$(err)" "✓  stay awake not set up" "state"
  assert_contains "$(err)" "stay-awake install --full --apply to set it up later" "next action"
}

test_leaves_a_stay_awake_mode_that_is_already_set() {
  sandbox
  mkdir -p "$S/home/.config/domux"
  printf '[stay_awake]\nmode = "partial"\n' > "$CONFIG"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "stay-awake" "nothing run"
  assert_eq '[stay_awake]
mode = "partial"

[keys]
leader = "C-s"' "$(config)" "the mode is kept"
  assert_contains "$(err)" "✓  stay awake already set to partial in $CONFIG" "names the mode that is set"
}

test_prints_the_lid_command_when_there_is_no_terminal_to_ask_on() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "stay-awake" "asks nothing and runs nothing"
  assert_contains "$(err)" "stay-awake install --full --apply" "the command to run later"
}

test_says_how_to_set_up_stay_awake_later_on_linux_without_a_terminal() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  release v1.0.1 linux amd64
  run_install DOMUX_VERSION=v1.0.1
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(err)" "✓  stay awake not set up" "state"
  assert_contains "$(err)" "add mode = \"full\" under [stay_awake] in $CONFIG to set it up later" "next action"
  assert_not_contains "$(err)" "stay-awake install" "the lid needs no setup command on Linux"
  assert_not_contains "$(config)" "stay_awake" "nothing written"
}

test_writes_only_the_data_line_to_stdout() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_eq 1 "$(wc -l < "$S/out" | tr -d ' ')" "one stdout line"
}

test_prints_no_color_when_stderr_is_not_a_terminal() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_not_contains "$(err)" "$ESC" "no escape sequences in a redirected stream"
  assert_not_contains "$(err)" "$CR" "no carriage returns in a redirected stream"
}

test_prints_no_spinner_frames_when_stderr_is_not_a_terminal() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  FAKE_CURL_DELAY=0.3
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  if has_frame "$(err)"; then fail "a spinner frame in a redirected stream: $(err)"; fi
  assert_not_contains "$(err)" "$ESC" "no escape sequences"
}

test_turns_a_spinner_while_the_network_steps_run_on_a_terminal() {
  sandbox
  on_a_terminal || return 0
  releases v1.0.0
  release v1.0.0 darwin arm64
  FAKE_CURL_DELAY=0.5
  run_install_tty DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(tty_text)"
  if has_frame "$(tty_text)"; then :; else fail "no spinner frame: $(tty_text)"; fi
  assert_contains "$(tty_text)" "looking up the newest release" "the lookup turns"
  assert_contains "$(tty_text)" "downloading domux 1.0.0" "the download turns"
  assert_contains "$(tty_text)" "$HIDE" "the cursor is hidden while it turns"
  last_hide=$(tty_text)
  last_hide=${last_hide##*"$HIDE"}
  assert_contains "$last_hide" "$SHOW" "the cursor is shown again"
  assert_contains "$(tty_text)" "domux is ready" "the steps still end"
}

test_prints_no_frame_for_a_step_that_does_not_wait_on_the_network() {
  sandbox
  on_a_terminal || return 0
  mkdir -p "$S/home/.claude" "$S/home/.codex"
  releases v1.0.0
  release v1.0.0 darwin arm64
  FAKE_DOMUX_DELAY=0.5
  run_install_tty DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(tty_text)"
  after=$(tty_text)
  after=${after##*"installed to"}
  assert_contains "$after" "detected, hooks installed" "the slow hooks ran"
  if has_frame "$after"; then fail "a frame after the network steps: $after"; fi
}

test_prints_the_same_lines_under_no_color_on_a_terminal() {
  sandbox
  on_a_terminal || return 0
  releases v1.0.0
  release v1.0.0 darwin arm64
  FAKE_CURL_DELAY=0.3
  run_install DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  cp "$S/err" "$S/plain"
  rm -rf "$S/home/.config" "$S/bin/domux"
  run_install_tty NO_COLOR=1 DOMUX_LEADER=C-a DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_not_contains "$(tty_text)" "$ESC" "no escape sequences"
  if has_frame "$(tty_text)"; then fail "a spinner frame under NO_COLOR: $(tty_text)"; fi
  assert_eq "$(cat "$S/plain")" "$(tr -d '\r' < "$S/tty")" "the lines a redirected stream gets"
}

test_clears_the_spinner_when_a_download_fails_on_a_terminal() {
  sandbox
  on_a_terminal || return 0
  releases v1.0.0
  release v1.0.0 darwin arm64
  FAKE_CURL_DELAY=0.5
  FAKE_CURL_FAIL="SHA256SUMS"
  run_install_tty DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=no
  assert_exit 1 "$code" "exit"
  after=$(tty_text)
  after=${after##*"downloading domux 1.0.0$EOL"}
  case $after in
    "$CR$EOL"*) ;;
    *) fail "the spinner line is not cleared after its last frame: $after" ;;
  esac
  assert_contains "$after" "$SHOW" "the cursor is shown again"
  assert_contains "$after" "curl: (22) The requested URL returned error: 404" "what curl said"
  assert_contains "$after" "could not download https://github.com/pranav7/domux/releases/download/v1.0.0/SHA256SUMS" "state"
  assert_contains "$after" "  next: the release is incomplete" "next action"
  if has_frame "$after"; then fail "a frame after the failure: $after"; fi
  assert_no_file "$S/bin/domux" "nothing installed"
}

# stops_on_signal <signal> <exit code>: sends the signal to the installer while a slow request
# runs on a terminal, and checks that the request, the spinner and the temp dir all went with it.
stops_on_signal() {
  sandbox
  on_a_terminal || return 0
  releases v1.0.0
  release v1.0.0 darwin arm64
  FAKE_CURL_DELAY=5
  : > "$S/keys"
  start_install_tty DOMUX_LEADER=C-s DOMUX_STAY_AWAKE=no
  if wait_for "$FAKE_HTTP_DIR/curl.pids" " " && wait_for "$S/tty" "looking up the newest release"; then
    read -r curl_pid installer_pid < "$FAKE_HTTP_DIR/curl.pids"
    kill "-$1" "$installer_pid"
    wait "$TTY_PID"
    code=$?
    assert_exit "$2" "$code" "exit"
    if kill -0 "$curl_pid" 2>/dev/null; then
      fail "the request the installer started is still running"
      kill "$curl_pid" 2>/dev/null
    fi
    assert_contains "$(tty_text)" "$HIDE" "the cursor was hidden while the request ran"
    last_hide=$(tty_text)
    last_hide=${last_hide##*"$HIDE"}
    assert_contains "$last_hide" "$SHOW" "the cursor is shown again"
    assert_eq "" "$(ls "$S/tmp")" "temp dir removed"
    assert_no_file "$S/bin/domux" "nothing installed"
  else
    fail "the request never started: $(tty_text)"
    kill "$TTY_PID" 2>/dev/null
  fi
}

test_stops_the_request_and_restores_the_cursor_on_term() {
  stops_on_signal TERM 143
}

test_stops_the_request_and_restores_the_cursor_on_int() {
  if [ -z "$DEFAULT_INT" ]; then
    printf 'skip %s: needs GNU env to give the installer SIGINT\n' "$CURRENT" >&2
    return 0
  fi
  stops_on_signal INT 130
}

# answer_on_a_terminal [<text> <keys>]...: runs the installer on Linux on a terminal with nothing
# preset, and types each answer once the text before it is on the screen. Sets code.
answer_on_a_terminal() {
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  releases v1.0.0
  release v1.0.0 linux amd64
  rm -f "$S/keys"
  mkfifo "$S/keys"
  start_install_tty
  exec 3>"$S/keys"
  while [ $# -ge 2 ]; do
    if ! type_after "$1" "$2"; then
      kill "$TTY_PID" 2>/dev/null
      break
    fi
    shift 2
  done
  wait "$TTY_PID"
  code=$?
  exec 3>&-
}

test_writes_the_leader_picked_from_the_list_on_a_terminal() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " 2 "put this machine to sleep?" y
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_contains "$(tty_text)" "Select your leader" "the question"
  assert_contains "$(tty_text)" "C-Space" "the list"
  assert_eq '[keys]
leader = "C-a"

[stay_awake]
mode = "full"' "$(config)" "the config file"
  assert_contains "$(tty_text)" "press C-a then A inside domux" "the leader picked"
}

test_writes_the_default_leader_when_enter_is_pressed_on_a_terminal() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " "$CR" "put this machine to sleep?" n
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_eq '[keys]
leader = "C-s"' "$(config)" "the default, and no stay awake mode"
}

test_writes_a_leader_typed_after_picking_another_key_on_a_terminal() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " 5 "key name  " "ctrl-a$CR" \
    "ctrl-a is not a key name" "M-a$CR" "put this machine to sleep?" n
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_contains "$(tty_text)" "ctrl-a is not a key name; write one like C-s, M-a or C-Space" "a name that is not a key is refused"
  assert_eq '[keys]
leader = "M-a"' "$(config)" "the typed leader"
}

test_takes_c_s_pressed_at_the_leader_list_without_pausing_the_terminal() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " '\023' "put this machine to sleep?" n
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_eq '[keys]
leader = "C-s"' "$(config)" "the key pressed"
  assert_contains "$(tty_text)" "domux is ready" "the install carries on"
}

test_takes_c_space_pressed_at_the_leader_list() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " '\0' "put this machine to sleep?" n
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_eq '[keys]
leader = "C-Space"' "$(config)" "the key pressed"
  assert_not_contains "$(tty_text)" "null byte" "no warning from the shell"
}

test_refuses_a_control_key_typed_as_a_key_name_without_pausing_the_terminal() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " 5 "key name  " '\023\r' \
    "like C-s, M-a or C-Space" 'M-a\r' "put this machine to sleep?" n
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_eq '[keys]
leader = "M-a"' "$(config)" "the typed leader"
}

test_ignores_keys_typed_before_a_question_is_asked() {
  sandbox
  on_a_terminal || return 0
  FAKE_CURL_DELAY=1
  answer_on_a_terminal "downloading domux 1.0.0" '\r\r' "or enter for C-s  " 2 "put this machine to sleep?" y
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_eq '[keys]
leader = "C-a"

[stay_awake]
mode = "full"' "$(config)" "the answers typed after each question"
}

test_ignores_an_arrow_key_at_the_stay_awake_question() {
  sandbox
  on_a_terminal || return 0
  answer_on_a_terminal "or enter for C-s  " '\r' "put this machine to sleep?" '\033[A' \
    "put this machine to sleep?" y
  assert_exit 0 "$code" "exit: $(tty_text)"
  assert_eq '[keys]
leader = "C-s"

[stay_awake]
mode = "full"' "$(config)" "the y after the arrow"
}

run_tests \
  test_installs_the_newest_stable_release_on_macos_arm64 \
  test_prints_the_logo_before_anything_else \
  test_marks_every_finished_step_with_a_tick \
  test_falls_back_to_a_prerelease_when_no_stable_release_exists \
  test_installs_a_pinned_version_on_linux_amd64 \
  test_maps_aarch64_to_arm64_and_accepts_a_version_without_v \
  test_maps_amd64_to_amd64_on_macos \
  test_installs_into_domux_install_dir \
  test_prints_the_path_line_when_the_install_dir_is_not_on_path \
  test_prints_no_path_line_when_the_install_dir_is_on_path \
  test_never_edits_shell_startup_files \
  test_replaces_an_existing_binary \
  test_fails_on_an_unsupported_os \
  test_fails_on_an_unsupported_arch \
  test_fails_on_musl \
  test_fails_when_curl_is_missing \
  test_fails_when_the_release_list_cannot_be_fetched \
  test_fails_when_no_release_exists \
  test_fails_when_domux_version_is_a_v1_go_tag \
  test_fails_when_domux_version_is_not_a_tag \
  test_fails_before_any_request_when_domux_leader_is_not_a_key_name \
  test_fails_when_the_archive_is_missing_from_the_release \
  test_fails_when_checksums_are_missing \
  test_fails_when_checksums_have_no_entry_for_the_archive \
  test_fails_when_the_checksum_mismatches \
  test_fails_when_the_archive_has_no_binary \
  test_fails_when_the_install_dir_is_not_writable \
  test_fails_when_the_installed_binary_does_not_run \
  test_says_each_agent_it_detected_and_installs_its_hooks \
  test_says_no_agent_was_detected_when_none_is_configured \
  test_follows_claude_config_dir_for_the_claude_hooks \
  test_says_what_happened_when_a_hook_install_fails \
  test_skips_the_hooks_when_asked \
  test_writes_the_leader_into_a_config_file_that_does_not_exist_yet \
  test_adds_the_leader_to_a_config_file_with_other_tables \
  test_adds_the_leader_under_keys_when_keys_has_no_leader \
  test_leaves_a_leader_that_is_already_set_and_says_which \
  test_writes_to_domux_config_file \
  test_writes_the_default_leader_when_there_is_no_terminal_to_ask_on \
  test_escapes_a_leader_that_toml_would_misread \
  test_writes_through_a_linked_config_file_and_keeps_the_link \
  test_leaves_a_config_file_that_sets_keys_without_a_table_alone \
  test_reads_a_header_inside_a_multi_line_array_or_string_as_part_of_it \
  test_adds_the_leader_under_a_keys_header_written_with_quotes \
  test_adds_the_leader_under_a_header_after_a_byte_order_mark \
  test_reads_a_leader_under_a_quoted_key_as_set \
  test_reads_a_leader_in_a_multi_line_string_as_set \
  test_reads_a_leader_that_is_a_question_mark_or_an_exclamation_mark_as_set \
  test_leaves_a_read_only_config_file_alone_without_a_shell_error \
  test_replaces_a_leftover_temp_file_link_rather_than_writing_through_it \
  test_turns_on_full_stay_awake_on_linux_without_sudo \
  test_names_the_leader_already_set_for_turning_stay_awake_on \
  test_sets_up_the_lid_and_writes_full_mode_on_macos_when_the_answer_is_yes \
  test_writes_no_stay_awake_mode_when_the_macos_lid_setup_fails \
  test_writes_no_stay_awake_mode_when_the_answer_is_no \
  test_leaves_a_stay_awake_mode_that_is_already_set \
  test_prints_the_lid_command_when_there_is_no_terminal_to_ask_on \
  test_says_how_to_set_up_stay_awake_later_on_linux_without_a_terminal \
  test_writes_only_the_data_line_to_stdout \
  test_prints_no_color_when_stderr_is_not_a_terminal \
  test_prints_no_spinner_frames_when_stderr_is_not_a_terminal \
  test_turns_a_spinner_while_the_network_steps_run_on_a_terminal \
  test_prints_no_frame_for_a_step_that_does_not_wait_on_the_network \
  test_prints_the_same_lines_under_no_color_on_a_terminal \
  test_clears_the_spinner_when_a_download_fails_on_a_terminal \
  test_stops_the_request_and_restores_the_cursor_on_term \
  test_stops_the_request_and_restores_the_cursor_on_int \
  test_writes_the_leader_picked_from_the_list_on_a_terminal \
  test_writes_the_default_leader_when_enter_is_pressed_on_a_terminal \
  test_writes_a_leader_typed_after_picking_another_key_on_a_terminal \
  test_takes_c_s_pressed_at_the_leader_list_without_pausing_the_terminal \
  test_takes_c_space_pressed_at_the_leader_list \
  test_refuses_a_control_key_typed_as_a_key_name_without_pausing_the_terminal \
  test_ignores_keys_typed_before_a_question_is_asked \
  test_ignores_an_arrow_key_at_the_stay_awake_question
