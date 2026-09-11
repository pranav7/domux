#!/bin/sh
# Smoke-tests one release archive on this machine. Extracts it into a temp directory, checks the
# license files are present, checks the signature on macOS, runs `domux --version` and
# `domux api schema` with a throwaway HOME and XDG_RUNTIME_DIR, no terminal and no server, and
# checks that neither started one. Never touches the caller's state, config or socket.
#
# `api schema` is the second check because it answers from the build itself rather than from a
# running server, so it proves the binary loads and its control API is intact without starting
# anything.
# Usage: scripts/release/smoke.sh <archive.tar.gz> <version>
set -eu

fail() { printf 'smoke: %s\n' "$1" >&2; exit 1; }

archive=${1:?usage: smoke.sh <archive> <version>}
version=${2:?usage: smoke.sh <archive> <version>}
[ -f "$archive" ] || fail "$archive does not exist"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM
mkdir "$work/bin" "$work/home" "$work/run"
tar -xzf "$archive" -C "$work/bin"

for f in domux LICENSE NOTICE THIRD_PARTY_LICENSES.md; do
  [ -f "$work/bin/$f" ] || fail "$f is missing from $archive"
done
[ -x "$work/bin/domux" ] || fail "domux in $archive is not executable"

# The archive's own name says which platform it is for, so a macOS archive is checked for its
# signature wherever it is smoke-tested, and a Linux archive is never asked for one.
case $(basename "$archive") in
  *_darwin_*)
    codesign --verify --verbose=2 "$work/bin/domux" 2>/dev/null || fail "domux in $archive has no valid code signature"
    ;;
esac

out=$(HOME="$work/home" TERM=dumb XDG_RUNTIME_DIR="$work/run" "$work/bin/domux" --version </dev/null 2>"$work/version.err") \
  || { cat "$work/version.err" >&2; fail "domux --version failed for $archive"; }
case $out in
  "domux $version"|"domux $version "*) ;;
  *) fail "domux --version printed \"$out\", want \"domux $version\"" ;;
esac

set +e
HOME="$work/home" TERM=dumb XDG_RUNTIME_DIR="$work/run" "$work/bin/domux" api schema </dev/null >"$work/schema.out" 2>"$work/schema.err"
code=$?
set -e
if [ "$code" != 0 ]; then
  cat "$work/schema.err" >&2
  fail "domux api schema exited $code for $archive"
fi
[ -s "$work/schema.out" ] || fail "domux api schema printed nothing"
grep -q '"methods"' "$work/schema.out" || fail "domux api schema printed no methods for $archive"
[ ! -e "$work/run/domux.sock" ] || fail "domux api schema created a socket at $work/run/domux.sock; it must not start the server"

printf 'smoke: %s ok (%s)\n' "$(basename "$archive")" "$out" >&2
