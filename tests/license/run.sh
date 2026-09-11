#!/bin/sh
# Checks the license files at the repository root. Run: sh tests/license/run.sh
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=tests/lib/assert.sh
. tests/lib/assert.sh

test_license_is_the_apache_2_text() {
  assert_eq "ApacheLicense" "$(sed -n '/[^ ]/{p;q;}' LICENSE | tr -d ' ')" "first non-blank line of LICENSE"
  assert_contains "$(sed -n 1,3p LICENSE)" "Version 2.0, January 2004" "LICENSE header"
  assert_contains "$(cat LICENSE)" "APPENDIX: How to apply the Apache License to your work." "LICENSE appendix"
  assert_not_contains "$(cat LICENSE)" "Permission is hereby granted, free of charge" "LICENSE must not be the MIT text"
}

test_notice_names_domux_and_its_copyright() {
  assert_eq "domux" "$(sed -n 1p NOTICE)" "NOTICE line 1"
  assert_contains "$(sed -n 2p NOTICE)" "Copyright 2026 Pranav Singh" "NOTICE line 2"
}

test_notice_reproduces_the_ghostty_license() {
  assert_contains "$(cat NOTICE)" "Permission is hereby granted, free of charge" "Ghostty MIT grant"
  assert_contains "$(cat NOTICE)" "THIRD_PARTY_LICENSES.md" "pointer to the generated attribution file"
  # The Ghostty source is a cache, not a checked-in file, so compare against it only when the
  # build has already cloned it. Without it, NOTICE alone is what ships and what is checked.
  src=$(crates/domux-term/scripts/ghostty-src.sh 2>/dev/null) || {
    echo "skip: no resolved ghostty tree; run cargo build -p domux-term --locked to compare" >&2
    return 0
  }
  copyright=$(grep -m1 '^Copyright' "$src/LICENSE")
  [ -n "$copyright" ] || fail "$src/LICENSE has no Copyright line"
  assert_contains "$(cat NOTICE)" "$copyright" "Ghostty copyright line"
}

test_workspace_license_is_apache_2() {
  table=$(awk '/^\[/ { in_pkg = ($0 == "[workspace.package]") } in_pkg { print }' Cargo.toml)
  assert_contains "$table" 'license = "Apache-2.0"' "[workspace.package] license"
}

test_every_crate_inherits_the_workspace_license() {
  for manifest in crates/*/Cargo.toml; do
    grep -q '^license.workspace = true' "$manifest" || fail "$manifest does not set license.workspace = true"
  done
}

test_about_config_accepts_no_copyleft() {
  accepted=$(awk '/^accepted *= *\[/,/^\]/' about.toml)
  assert_contains "$accepted" '"Apache-2.0"' "Apache-2.0 accepted"
  assert_contains "$accepted" '"MIT"' "MIT accepted"
  for bad in GPL LGPL AGPL SSPL; do
    assert_not_contains "$accepted" "$bad" "$bad must not be accepted"
  done
}

run_tests \
  test_license_is_the_apache_2_text \
  test_notice_names_domux_and_its_copyright \
  test_notice_reproduces_the_ghostty_license \
  test_workspace_license_is_apache_2 \
  test_every_crate_inherits_the_workspace_license \
  test_about_config_accepts_no_copyleft
