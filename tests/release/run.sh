#!/bin/sh
# Tests for the release scripts under scripts/release/. Run: sh tests/release/run.sh
# Cargo and cargo-about are never invoked: build-archive.sh takes the binary from
# DOMUX_RELEASE_BIN and the attribution file from DOMUX_RELEASE_THIRD_PARTY. Every test runs in
# its own sandbox directory holding a small Cargo.toml, CHANGELOG.md, LICENSE and NOTICE.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=tests/lib/assert.sh
. tests/lib/assert.sh
ROOT=$(pwd)
HOST=$(uname -s)
SANDBOX_ROOT=$(mktemp -d)
trap 'rm -rf "$SANDBOX_ROOT"' EXIT INT TERM

# sandbox: a fresh directory with the files the scripts read; leaves the shell inside it.
sandbox() {
  S="$SANDBOX_ROOT/$CURRENT"
  mkdir -p "$S"
  cd "$S" || exit 1
  cp "$ROOT/LICENSE" "$ROOT/NOTICE" .
  printf '# Third-party licenses (test stand-in)\n' > third-party.md
  cat > Cargo.toml <<'TOML'
[workspace]
members = ["crates/domux"]

[workspace.package]
version = "1.0.0"
edition = "2021"

[profile.release]
lto = "fat"
TOML
  cat > CHANGELOG.md <<'MD'
# Changelog

## [Unreleased]

- Not yet released.

## [1.0.0] - 2026-09-20

### Added

- Everything in the first release.

## [1.0.0-beta.1] - 2026-09-10

- Beta.

## [1.0.0-alpha.1] - 2026-09-01

[1.0.0]: https://github.com/pranav7/domux/compare/v1.0.0-beta.1...v1.0.0
[1.0.0-beta.1]: https://github.com/pranav7/domux/compare/v1.0.0-alpha.1...v1.0.0-beta.1
[1.0.0-alpha.1]: https://github.com/pranav7/domux/releases/tag/v1.0.0-alpha.1
MD
}

# fake_domux <path> <version> [api schema exit code] [schema body]: a stand-in binary that
# answers the two commands smoke.sh runs.
fake_domux() {
  fd_path=$1
  fd_version=$2
  fd_code=${3:-0}
  fd_body=${4:-'{"methods": []}'}
  # shellcheck disable=SC2016  # these are the lines of the generated script, not expansions here
  {
    printf '#!/bin/sh\n'
    printf 'if [ "${1:-}" = --version ]; then printf "domux %s\\n"; exit 0; fi\n' "$fd_version"
    printf 'if [ "${1:-}" = api ] && [ "${2:-}" = schema ]; then printf "%%s\\n" %s; exit %s; fi\n' \
      "'$fd_body'" "$fd_code"
    printf 'exit 2\n'
  } > "$fd_path"
  chmod 0755 "$fd_path"
}

# archive <target> <version> [binary]: runs build-archive.sh with the fakes; leaves output in out/err.
archive() {
  DOMUX_RELEASE_BIN="${3:-fake-domux}" DOMUX_RELEASE_THIRD_PARTY=third-party.md \
    sh "$ROOT/scripts/release/build-archive.sh" "$1" "$2" dist >out 2>err
}

test_check_version_prints_the_body_when_the_tag_matches() {
  sandbox
  out=$(sh "$ROOT/scripts/release/check-version.sh" v1.0.0 Cargo.toml 2>err); code=$?
  assert_exit 0 "$code" "exit"
  assert_eq "1.0.0" "$out" "stdout"
  assert_eq "" "$(cat err)" "stderr"
}

test_check_version_accepts_a_prerelease_tag() {
  sandbox
  sed -i.bak 's/^version = "1.0.0"/version = "1.0.0-beta.1"/' Cargo.toml
  out=$(sh "$ROOT/scripts/release/check-version.sh" v1.0.0-beta.1 Cargo.toml 2>err); code=$?
  assert_exit 0 "$code" "exit"
  assert_eq "1.0.0-beta.1" "$out" "stdout"
}

test_check_version_rejects_a_mismatched_tag() {
  sandbox
  out=$(sh "$ROOT/scripts/release/check-version.sh" v1.1.0 Cargo.toml 2>err); code=$?
  assert_exit 1 "$code" "exit"
  assert_eq "" "$out" "stdout"
  assert_contains "$(cat err)" "check-version: tag v1.1.0 does not match Cargo.toml version 1.0.0" "state"
  assert_contains "$(cat err)" "  next: set version = \"1.1.0\" in Cargo.toml" "next action"
}

test_check_version_rejects_a_v1_go_tag() {
  sandbox
  sh "$ROOT/scripts/release/check-version.sh" v0.4.0 Cargo.toml >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "v0.4.0 is a domux V1 tag" "state"
}

test_check_version_rejects_a_tag_that_is_not_a_version() {
  sandbox
  sh "$ROOT/scripts/release/check-version.sh" latest Cargo.toml >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "latest is not a release tag" "state"
}

test_check_version_fails_without_a_workspace_version() {
  sandbox
  printf '[workspace]\nmembers = []\n' > Cargo.toml
  sh "$ROOT/scripts/release/check-version.sh" v1.0.0 Cargo.toml >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "no version under [workspace.package] in Cargo.toml" "state"
}

test_changelog_section_prints_one_version() {
  sandbox
  out=$(sh "$ROOT/scripts/release/changelog-section.sh" 1.0.0 CHANGELOG.md 2>err); code=$?
  assert_exit 0 "$code" "exit"
  assert_contains "$out" "Everything in the first release." "own content"
  assert_contains "$out" "### Added" "own subheading"
  assert_not_contains "$out" "Not yet released" "unreleased content"
  assert_not_contains "$out" "Beta." "next section"
  assert_not_contains "$out" "## [" "version headings"
  assert_not_contains "$out" "https://github.com" "link references"
}

test_changelog_section_prints_a_prerelease_version() {
  sandbox
  out=$(sh "$ROOT/scripts/release/changelog-section.sh" 1.0.0-beta.1 CHANGELOG.md 2>err); code=$?
  assert_exit 0 "$code" "exit"
  assert_eq "- Beta." "$(printf '%s' "$out" | sed '/^$/d')" "body without blank lines"
}

test_changelog_section_fails_when_the_section_is_missing() {
  sandbox
  sh "$ROOT/scripts/release/changelog-section.sh" 9.9.9 CHANGELOG.md >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_eq "" "$(cat out)" "stdout"
  assert_contains "$(cat err)" 'changelog-section: CHANGELOG.md has no section "## [9.9.9]" with content' "state"
}

test_changelog_section_fails_when_the_section_is_empty() {
  sandbox
  sh "$ROOT/scripts/release/changelog-section.sh" 1.0.0-alpha.1 CHANGELOG.md >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" 'no section "## [1.0.0-alpha.1]" with content' "state"
}

test_build_archive_packs_and_names_linux_targets() {
  sandbox
  fake_domux fake-domux 1.0.0
  archive x86_64-unknown-linux-gnu 1.0.0; code=$?
  assert_exit 0 "$code" "exit for x86_64: $(cat err)"
  archive aarch64-unknown-linux-gnu 1.0.0; code=$?
  assert_exit 0 "$code" "exit for aarch64: $(cat err)"
  assert_file dist/domux_1.0.0_linux_amd64.tar.gz "amd64 archive"
  assert_file dist/domux_1.0.0_linux_arm64.tar.gz "arm64 archive"
  assert_eq "LICENSE
NOTICE
THIRD_PARTY_LICENSES.md
domux" "$(tar -tzf dist/domux_1.0.0_linux_amd64.tar.gz | LC_ALL=C sort)" "archive members"
  sum_line=$(cat dist/domux_1.0.0_linux_amd64.sha256)
  assert_contains "$sum_line" "  domux_1.0.0_linux_amd64.tar.gz" "checksum line names the archive after two spaces"
  assert_eq 64 "$(printf '%s' "$sum_line" | cut -d' ' -f1 | tr -d '\n' | wc -c | tr -d ' ')" "hash length"
  mkdir x && tar -xzf dist/domux_1.0.0_linux_amd64.tar.gz -C x
  assert_eq "domux 1.0.0" "$(x/domux --version)" "packed binary runs"
  [ -x x/domux ] || fail "packed binary is not executable"
  assert_eq "" "$(cat out)" "stdout stays empty"
}

test_build_archive_signs_macos_targets_on_macos_and_refuses_them_elsewhere() {
  sandbox
  # A real Mach-O binary, because codesign stores a script's signature in an extended attribute
  # that tar does not carry, while a Mach-O binary keeps it inside the file. That is the property
  # the release archive depends on, so the test packs something that has it.
  cp /bin/echo fake-domux
  archive aarch64-apple-darwin 1.0.0; code=$?
  if [ "$HOST" = Darwin ]; then
    assert_exit 0 "$code" "exit: $(cat err)"
    assert_file dist/domux_1.0.0_darwin_arm64.tar.gz "arm64 archive"
    archive x86_64-apple-darwin 1.0.0; code=$?
    assert_exit 0 "$code" "exit for x86_64: $(cat err)"
    assert_file dist/domux_1.0.0_darwin_amd64.tar.gz "amd64 archive"
    mkdir x && tar -xzf dist/domux_1.0.0_darwin_arm64.tar.gz -C x
    codesign --verify --verbose=2 x/domux 2>/dev/null || fail "packed binary has no valid signature"
  else
    assert_exit 1 "$code" "exit"
    assert_contains "$(cat err)" "build-archive: macOS archives are built on macOS" "state"
  fi
}

test_build_archive_rejects_an_unknown_target() {
  sandbox
  fake_domux fake-domux 1.0.0
  archive x86_64-pc-windows-msvc 1.0.0; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "build-archive: no release mapping for target x86_64-pc-windows-msvc" "state"
  assert_no_file dist "no dist directory"
}

test_build_archive_fails_when_the_binary_is_missing() {
  sandbox
  archive x86_64-unknown-linux-gnu 1.0.0 missing-binary; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "build-archive: DOMUX_RELEASE_BIN missing-binary is not a file" "state"
}

test_smoke_passes_on_a_good_archive() {
  sandbox
  fake_domux fake-domux 1.0.0
  archive x86_64-unknown-linux-gnu 1.0.0
  sh "$ROOT/scripts/release/smoke.sh" dist/domux_1.0.0_linux_amd64.tar.gz 1.0.0 >out 2>err; code=$?
  assert_exit 0 "$code" "exit: $(cat err)"
  assert_contains "$(cat err)" "smoke: domux_1.0.0_linux_amd64.tar.gz ok (domux 1.0.0)" "summary"
  assert_eq "" "$(cat out)" "stdout stays empty"
}

test_smoke_fails_when_the_version_differs() {
  sandbox
  fake_domux fake-domux 1.0.0
  archive x86_64-unknown-linux-gnu 1.0.0
  sh "$ROOT/scripts/release/smoke.sh" dist/domux_1.0.0_linux_amd64.tar.gz 1.0.1 >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" 'smoke: domux --version printed "domux 1.0.0", want "domux 1.0.1"' "state"
}

test_smoke_fails_when_the_schema_call_fails() {
  sandbox
  fake_domux fake-domux 1.0.0 3
  archive x86_64-unknown-linux-gnu 1.0.0
  sh "$ROOT/scripts/release/smoke.sh" dist/domux_1.0.0_linux_amd64.tar.gz 1.0.0 >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "smoke: domux api schema exited 3" "state"
}

test_smoke_fails_when_the_schema_has_no_methods() {
  sandbox
  fake_domux fake-domux 1.0.0 0 '{}'
  archive x86_64-unknown-linux-gnu 1.0.0
  sh "$ROOT/scripts/release/smoke.sh" dist/domux_1.0.0_linux_amd64.tar.gz 1.0.0 >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "printed no methods" "state"
}

test_smoke_fails_when_the_binary_starts_a_server() {
  sandbox
  cat > fake-domux <<'SH'
#!/bin/sh
if [ "${1:-}" = --version ]; then printf 'domux 1.0.0\n'; exit 0; fi
if [ "${1:-}" = api ] && [ "${2:-}" = schema ]; then
  mkdir -p "$XDG_RUNTIME_DIR" && : > "$XDG_RUNTIME_DIR/domux.sock"
  printf '{"methods": []}\n'
  exit 0
fi
exit 2
SH
  chmod 0755 fake-domux
  archive x86_64-unknown-linux-gnu 1.0.0
  sh "$ROOT/scripts/release/smoke.sh" dist/domux_1.0.0_linux_amd64.tar.gz 1.0.0 >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "created a socket" "state"
}

test_smoke_fails_when_a_license_file_is_missing() {
  sandbox
  fake_domux domux 1.0.0
  mkdir dist
  tar -czf dist/domux_1.0.0_linux_amd64.tar.gz domux LICENSE
  sh "$ROOT/scripts/release/smoke.sh" dist/domux_1.0.0_linux_amd64.tar.gz 1.0.0 >out 2>err; code=$?
  assert_exit 1 "$code" "exit"
  assert_contains "$(cat err)" "smoke: NOTICE is missing from dist/domux_1.0.0_linux_amd64.tar.gz" "state"
}

run_tests \
  test_check_version_prints_the_body_when_the_tag_matches \
  test_check_version_accepts_a_prerelease_tag \
  test_check_version_rejects_a_mismatched_tag \
  test_check_version_rejects_a_v1_go_tag \
  test_check_version_rejects_a_tag_that_is_not_a_version \
  test_check_version_fails_without_a_workspace_version \
  test_changelog_section_prints_one_version \
  test_changelog_section_prints_a_prerelease_version \
  test_changelog_section_fails_when_the_section_is_missing \
  test_changelog_section_fails_when_the_section_is_empty \
  test_build_archive_packs_and_names_linux_targets \
  test_build_archive_signs_macos_targets_on_macos_and_refuses_them_elsewhere \
  test_build_archive_rejects_an_unknown_target \
  test_build_archive_fails_when_the_binary_is_missing \
  test_smoke_passes_on_a_good_archive \
  test_smoke_fails_when_the_version_differs \
  test_smoke_fails_when_the_schema_call_fails \
  test_smoke_fails_when_the_schema_has_no_methods \
  test_smoke_fails_when_the_binary_starts_a_server \
  test_smoke_fails_when_a_license_file_is_missing
