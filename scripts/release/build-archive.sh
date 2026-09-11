#!/bin/sh
# Builds one release archive for one cargo target.
# Usage: scripts/release/build-archive.sh <cargo-target> <version> [dist]
# Writes <dist>/domux_<version>_<os>_<arch>.tar.gz holding domux, LICENSE, NOTICE and
# THIRD_PARTY_LICENSES.md at the top level, and <dist>/domux_<version>_<os>_<arch>.sha256 with
# one line in shasum format: "<hex>  <archive file name>".
#
# The target mapping mirrors crates/domux-term/build.rs and the names mirror V1's releases.
# DOMUX_RELEASE_BIN=<path> packs that binary instead of running cargo (tests use it).
# DOMUX_RELEASE_THIRD_PARTY=<path> packs that file instead of running cargo-about (tests use it).
set -eu

fail() { printf 'build-archive: %s\n  next: %s\n' "$1" "$2" >&2; exit 1; }

target=${1:?usage: build-archive.sh <cargo-target> <version> [dist]}
version=${2:?usage: build-archive.sh <cargo-target> <version> [dist]}
dist=${3:-dist}

case $target in
  aarch64-apple-darwin)      os=darwin arch=arm64 ;;
  x86_64-apple-darwin)       os=darwin arch=amd64 ;;
  x86_64-unknown-linux-gnu)  os=linux  arch=amd64 ;;
  aarch64-unknown-linux-gnu) os=linux  arch=arm64 ;;
  *) fail "no release mapping for target $target" "use one of aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu" ;;
esac

if [ "$os" = darwin ] && [ "$(uname -s)" != Darwin ]; then
  fail "macOS archives are built on macOS, because codesign runs here; this host is $(uname -s)" "run this script on a macOS runner (the release workflow does)"
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT INT TERM

if [ -n "${DOMUX_RELEASE_BIN:-}" ]; then
  [ -f "$DOMUX_RELEASE_BIN" ] || fail "DOMUX_RELEASE_BIN $DOMUX_RELEASE_BIN is not a file" "point it at a built domux binary or unset it to let cargo build one"
  cp "$DOMUX_RELEASE_BIN" "$stage/domux"
else
  cargo build --release --locked -p domux --target "$target"
  cp "target/$target/release/domux" "$stage/domux"
fi
chmod 0755 "$stage/domux"

if [ "$os" = darwin ]; then
  # Apple silicon refuses to run arm64 code with no signature, and stripping can invalidate the
  # signature the linker added. The ad-hoc identity "-" needs no certificate. This is not a
  # Developer ID signature and the binary is not notarized; the README's macOS note covers what
  # that means for a browser download.
  codesign --force --sign - "$stage/domux"
  codesign --verify --verbose=2 "$stage/domux"
fi

cp LICENSE NOTICE "$stage/"
if [ -n "${DOMUX_RELEASE_THIRD_PARTY:-}" ]; then
  cp "$DOMUX_RELEASE_THIRD_PARTY" "$stage/THIRD_PARTY_LICENSES.md"
else
  cargo about generate --fail -o "$stage/THIRD_PARTY_LICENSES.md" about.hbs
fi

mkdir -p "$dist"
name="domux_${version}_${os}_${arch}"
# COPYFILE_DISABLE keeps macOS tar from adding ._ entries for extended attributes.
COPYFILE_DISABLE=1 tar -czf "$dist/$name.tar.gz" -C "$stage" domux LICENSE NOTICE THIRD_PARTY_LICENSES.md

if command -v sha256sum >/dev/null 2>&1; then
  sum=$(sha256sum "$dist/$name.tar.gz" | cut -d' ' -f1)
else
  sum=$(shasum -a 256 "$dist/$name.tar.gz" | cut -d' ' -f1)
fi
printf '%s  %s\n' "$sum" "$name.tar.gz" > "$dist/$name.sha256"
printf 'build-archive: wrote %s/%s.tar.gz\n' "$dist" "$name" >&2
