#!/bin/sh
# domux installer — downloads latest release binary into ~/.local/bin
# and runs `domux bootstrap` to wire up tmux/Claude/Codex/caffeinate.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
#
# Env vars:
#   DOMUX_VERSION         pin to a specific tag (e.g. v0.1.0); defaults to latest
#   DOMUX_INSTALL_DIR     override install dir (defaults to ~/.local/bin)
#   DOMUX_SKIP_BOOTSTRAP  set to 1 to skip the bootstrap handoff

set -eu

REPO="pranav7/domux"
INSTALL_DIR="${DOMUX_INSTALL_DIR:-$HOME/.local/bin}"

die() {
  printf '\033[31m✗\033[0m %s\n' "$*" >&2
  exit 1
}

info() {
  printf '\033[36m→\033[0m %s\n' "$*"
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

need curl
need tar
need uname

# --- detect OS/arch -----------------------------------------------------------

os="$(uname -s)"
case "$os" in
  Darwin) os="darwin" ;;
  *) die "domux currently supports macOS only (detected: $os)" ;;
esac

arch_raw="$(uname -m)"
case "$arch_raw" in
  arm64|aarch64) arch="arm64" ;;
  x86_64|amd64)  arch="amd64" ;;
  *) die "unsupported architecture: $arch_raw" ;;
esac

info "Detected $os $arch"

# --- resolve version ----------------------------------------------------------

version="${DOMUX_VERSION:-}"
if [ -z "$version" ]; then
  info "Resolving latest release …"
  version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep -m1 '"tag_name":' \
    | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')" \
    || die "could not resolve latest release"
  [ -n "$version" ] || die "could not parse latest release tag"
fi

# strip leading v for filename body
version_body="${version#v}"
tarball="domux_${version_body}_${os}_${arch}.tar.gz"
base_url="https://github.com/$REPO/releases/download/$version"

# --- download + verify --------------------------------------------------------

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

info "Downloading $tarball ($version) …"
curl -fsSL -o "$tmp/$tarball" "$base_url/$tarball" \
  || die "download failed: $base_url/$tarball"

info "Downloading checksums …"
curl -fsSL -o "$tmp/SHA256SUMS" "$base_url/SHA256SUMS" \
  || die "could not fetch SHA256SUMS"

info "Verifying SHA256 …"
( cd "$tmp" && grep " $tarball\$" SHA256SUMS | shasum -a 256 -c - >/dev/null ) \
  || die "checksum verification failed for $tarball"

# --- install ------------------------------------------------------------------

mkdir -p "$INSTALL_DIR"
tar -xzf "$tmp/$tarball" -C "$tmp"
[ -f "$tmp/domux" ] || die "tarball did not contain a domux binary"
mv "$tmp/domux" "$INSTALL_DIR/domux"
chmod 0755 "$INSTALL_DIR/domux"

info "Installed $INSTALL_DIR/domux ($version)"

# --- PATH hint ----------------------------------------------------------------

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    printf '\n\033[33m!\033[0m %s is not on your PATH. Add this to your shell rc:\n' "$INSTALL_DIR"
    printf '    export PATH="%s:$PATH"\n\n' "$INSTALL_DIR"
    ;;
esac

# --- bootstrap handoff --------------------------------------------------------

if [ "${DOMUX_SKIP_BOOTSTRAP:-}" = "1" ]; then
  info "Skipping bootstrap (DOMUX_SKIP_BOOTSTRAP=1). Run \`domux bootstrap\` later."
  exit 0
fi

if [ -r /dev/tty ]; then
  info "Running domux bootstrap …"
  exec "$INSTALL_DIR/domux" bootstrap </dev/tty
else
  info "Non-interactive shell — run \`domux bootstrap\` to finish setup."
fi
