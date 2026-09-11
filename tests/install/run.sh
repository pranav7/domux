#!/bin/sh
# Tests for install.sh, one per branch of the script. A fake curl, uname and ldd sit first on
# PATH; tar, mktemp and sha256sum or shasum are the real ones. Every test builds its own fake
# release under a sandbox and installs into a sandbox directory; nothing touches the real HOME.
# Run: sh tests/install/run.sh
# TEST_SHELL=bash sh tests/install/run.sh runs install.sh under bash instead of sh.
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

# sandbox: a fresh home, install dir, temp dir and fake http root for one test; macOS arm64 by default.
sandbox() {
  S="$SANDBOX_ROOT/$CURRENT"
  mkdir -p "$S/home" "$S/bin" "$S/tmp" "$S/http"
  FAKE_HTTP_DIR="$S/http"
  : > "$FAKE_HTTP_DIR/releases.json"
  : > "$FAKE_HTTP_DIR/requests.log"
  FAKE_CURL_FAIL=""
  FAKE_LDD=""
  FAKE_UNAME_S=Darwin
  FAKE_UNAME_M=arm64
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
# test can say which setup commands the installer ran.
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
    FAKE_UNAME_S="$FAKE_UNAME_S" FAKE_UNAME_M="$FAKE_UNAME_M" \
    DOMUX_INSTALL_DIR="$S/bin" "$@" "$TEST_SHELL" "$ROOT/install.sh" >"$S/out" 2>"$S/err"
  code=$?
}

err() { cat "$S/err"; }
out() { cat "$S/out"; }
requests() { cat "$FAKE_HTTP_DIR/requests.log"; }
calls() { cat "$S/calls.log" 2>/dev/null; }

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
  assert_contains "$(err)" "domux is ready." "ready line"
  assert_contains "$(err)" "domux install claude --apply" "next steps"
  assert_contains "$(err)" "macos on arm64" "platform line"
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
./.zshrc" "$(cd "$S/home" && find . -mindepth 1 -maxdepth 1 | LC_ALL=C sort)" "nothing else under HOME"
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

test_installs_hooks_for_every_agent_that_is_configured() {
  sandbox
  mkdir -p "$S/home/.claude" "$S/home/.config/opencode"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(calls)" "install claude --apply" "claude hooks"
  assert_contains "$(calls)" "install opencode --apply" "opencode hooks"
  assert_contains "$(err)" "hooks for claude opencode" "both agents on one line"
  assert_not_contains "$(calls)" "install codex --apply" "codex is not configured here"
  assert_contains "$(err)" "hooks for claude" "the agents that got hooks"
}

test_installs_no_hooks_when_no_agent_is_configured() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "--apply" "nothing applied"
  assert_contains "$(err)" "no agent is configured on this machine yet" "state"
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
  assert_contains "$(err)" "could not install the claude hooks" "state"
  assert_contains "$(err)" "install claude --apply to see what happened" "next action"
}

test_skips_the_hooks_when_asked() {
  sandbox
  mkdir -p "$S/home/.claude"
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_HOOKS=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "--apply" "nothing applied"
  assert_contains "$(err)" "hooks skipped" "state"
}

test_sets_up_the_lid_when_the_answer_is_yes() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_STAY_AWAKE=yes
  assert_exit 0 "$code" "exit: $(err)"
  assert_contains "$(calls)" "stay-awake install --full --apply" "the command it runs"
  assert_contains "$(err)" "the lid is covered too" "state"
  assert_contains "$(err)" 'mode = "full"' "the line to add to the config file"
}

test_skips_the_lid_when_the_answer_is_no() {
  sandbox
  releases v1.0.0
  release v1.0.0 darwin arm64
  run_install DOMUX_STAY_AWAKE=no
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(calls)" "stay-awake" "nothing run"
  assert_contains "$(err)" "the lid setup was skipped" "state"
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

test_says_nothing_about_the_lid_on_linux() {
  sandbox
  FAKE_UNAME_S=Linux; FAKE_UNAME_M=x86_64
  release v1.0.1 linux amd64
  run_install DOMUX_VERSION=v1.0.1
  assert_exit 0 "$code" "exit: $(err)"
  assert_not_contains "$(err)" "stay-awake" "the lid needs no setup on Linux"
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
  assert_not_contains "$(err)" "$(printf '\033')" "no escape sequences in a redirected stream"
}

run_tests \
  test_installs_the_newest_stable_release_on_macos_arm64 \
  test_prints_the_logo_before_anything_else \
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
  test_fails_when_the_archive_is_missing_from_the_release \
  test_fails_when_checksums_are_missing \
  test_fails_when_checksums_have_no_entry_for_the_archive \
  test_fails_when_the_checksum_mismatches \
  test_fails_when_the_archive_has_no_binary \
  test_fails_when_the_install_dir_is_not_writable \
  test_fails_when_the_installed_binary_does_not_run \
  test_installs_hooks_for_every_agent_that_is_configured \
  test_installs_no_hooks_when_no_agent_is_configured \
  test_follows_claude_config_dir_for_the_claude_hooks \
  test_says_what_happened_when_a_hook_install_fails \
  test_skips_the_hooks_when_asked \
  test_sets_up_the_lid_when_the_answer_is_yes \
  test_skips_the_lid_when_the_answer_is_no \
  test_prints_the_lid_command_when_there_is_no_terminal_to_ask_on \
  test_says_nothing_about_the_lid_on_linux \
  test_writes_only_the_data_line_to_stdout \
  test_prints_no_color_when_stderr_is_not_a_terminal
