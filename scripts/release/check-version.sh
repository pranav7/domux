#!/bin/sh
# Checks that a release tag equals the workspace version in Cargo.toml.
# Usage: scripts/release/check-version.sh <tag> [Cargo.toml]
# Prints the version body (the tag without its leading v) on stdout and exits 0 when they match.
# Otherwise prints the state and the next action on stderr and exits 1.
#
# domux releases are v1 and later. The v0.x tags are V1, the Go version on the v1 branch, and
# this script refuses them so a V1 tag can never publish a V2 archive.
set -eu

fail() { printf 'check-version: %s\n  next: %s\n' "$1" "$2" >&2; exit 1; }

tag=${1:?usage: check-version.sh <tag> [Cargo.toml]}
manifest=${2:-Cargo.toml}

case $tag in
  v0.*) fail "$tag is a domux V1 tag; this pipeline releases the Rust version" "tag as v1.<minor>.<patch> or later" ;;
  v[1-9]*) ;;
  *) fail "$tag is not a release tag" "tag as v<major>.<minor>.<patch> or v<major>.<minor>.<patch>-<prerelease>, with major 1 or greater" ;;
esac
body=${tag#v}

[ -f "$manifest" ] || fail "$manifest does not exist" "run from the repository root or pass the path to Cargo.toml"

version=$(awk '
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^version[ \t]*=/ { gsub(/.*=[ \t]*"|".*/, ""); print; exit }
' "$manifest")
[ -n "$version" ] || fail "no version under [workspace.package] in $manifest" "add version = \"<major.minor.patch>\" to that table"
[ "$version" = "$body" ] || fail "tag $tag does not match Cargo.toml version $version" "set version = \"$body\" in $manifest, commit, and tag that commit"

printf '%s\n' "$body"
