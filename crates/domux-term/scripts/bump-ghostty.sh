#!/bin/sh
# Bumps the pinned Ghostty commit, then does everything the new pin invalidates:
# regenerates the bindings from the new headers and runs the checks that must agree with
# them. Usage: crates/domux-term/scripts/bump-ghostty.sh <commit>
set -eu
cd "$(dirname "$0")/../../.."
COMMIT=${1:-}
case "$COMMIT" in
  ?????????????????????????????????????????) ;;
  *) echo "usage: $0 <full 40-character commit>" >&2; exit 1 ;;
esac
PIN=vendor/ghostty-pin.toml
OLD=$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$PIN")
[ "$OLD" != "$COMMIT" ] || { echo "already pinned to $COMMIT"; exit 0; }

sed "s/^commit = \".*\"$/commit = \"$COMMIT\"/" "$PIN" > "$PIN.tmp"
mv "$PIN.tmp" "$PIN"
echo "pin: $OLD -> $COMMIT"

# Builds the new commit, which clones it and fails if the commit does not exist.
cargo build -p domux-term --features ghostty
crates/domux-term/scripts/regen-ghostty-bindings.sh
cargo test --workspace --all-features

REPO=$(sed -n 's/^repo = "\(.*\)"$/\1/p' "$PIN" | sed 's/\.git$//')
echo
echo "review upstream: $REPO/compare/$OLD...$COMMIT"
echo "the two-line pin diff hides that range; read the build and header changes in it."
