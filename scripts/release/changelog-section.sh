#!/bin/sh
# Prints one version's CHANGELOG.md section on stdout: the lines between the heading
# "## [<version>] - <date>" and the next "## [" heading, without the link reference lines
# ("[1.0.0]: https://...") that Keep a Changelog puts at the end of the file.
# Usage: scripts/release/changelog-section.sh <version> [CHANGELOG.md]
# Exits 1 with the state and the next action on stderr when the section is missing or empty.
set -eu

fail() { printf 'changelog-section: %s\n  next: %s\n' "$1" "$2" >&2; exit 1; }

version=${1:?usage: changelog-section.sh <version> [CHANGELOG.md]}
changelog=${2:-CHANGELOG.md}
[ -f "$changelog" ] || fail "$changelog does not exist" "run from the repository root or pass the path"

section=$(awk -v v="$version" '
  /^## \[/ {
    if (found) exit
    if (index($0, "## [" v "]") == 1) { found = 1; next }
  }
  found && /^\[[^]]*\]: / { next }
  found { print }
' "$changelog")

if [ -z "$(printf '%s' "$section" | tr -d '[:space:]')" ]; then
  fail "$changelog has no section \"## [$version]\" with content" "write the section under the Keep a Changelog convention described at the top of $changelog, commit, then tag"
fi
printf '%s\n' "$section"
