#!/bin/sh
# Prints the Ghostty tree the build uses, resolved the same way build.rs resolves it:
# DOMUX_GHOSTTY_SOURCE_DIR when set, otherwise the pinned commit's clone under the cache.
set -eu
cd "$(dirname "$0")/../../.."
if [ -n "${DOMUX_GHOSTTY_SOURCE_DIR:-}" ]; then
  SRC="$DOMUX_GHOSTTY_SOURCE_DIR"
else
  COMMIT=$(sed -n 's/^commit = "\(.*\)"$/\1/p' vendor/ghostty-pin.toml)
  [ -n "$COMMIT" ] || { echo "no commit in vendor/ghostty-pin.toml" >&2; exit 1; }
  SRC="${XDG_CACHE_HOME:-$HOME/.cache}/domux/ghostty/$COMMIT"
fi
if [ ! -d "$SRC/include/ghostty" ]; then
  echo "no ghostty source at $SRC" >&2
  echo "run: cargo build -p domux-term --features ghostty" >&2
  exit 1
fi
printf '%s\n' "$SRC"
