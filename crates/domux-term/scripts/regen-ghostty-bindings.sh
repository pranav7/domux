#!/bin/sh
# Regenerates src/ghostty/ffi.rs from the pinned Ghostty headers and appends a fingerprint of
# them. tests/header_fingerprint.rs fails when those headers change without a rerun.
set -eu
cd "$(dirname "$0")/../../.."
OUT=crates/domux-term/src/ghostty/ffi.rs
SRC=$(crates/domux-term/scripts/ghostty-src.sh)

bindgen "$SRC/include/ghostty/vt.h" \
  --allowlist-function 'ghostty_.*' \
  --allowlist-type 'Ghostty.*' \
  --allowlist-var 'GHOSTTY_.*' \
  --raw-line '#![allow(non_camel_case_types, non_upper_case_globals, non_snake_case, dead_code, clippy::all)]' \
  -- -I "$SRC/include" > "$OUT.tmp"
printf '\n// header-sha256: %s\n' "$(find "$SRC/include/ghostty" -name '*.h' | LC_ALL=C sort | xargs cat | shasum -a 256 | cut -d' ' -f1)" >> "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "wrote $OUT from $SRC"
