#!/bin/sh
# domux installer. Downloads the release archive for this machine, verifies its checksum, and
# installs the domux binary. Nothing else is written and no shell startup file is edited.
#
#   curl -fsSL https://raw.githubusercontent.com/pranav7/domux/main/install.sh | sh
#
# Environment:
#   DOMUX_VERSION      release tag to install, for example v1.0.0; default: the newest release
#                      (stable first; a prerelease only when no stable release exists)
#   DOMUX_INSTALL_DIR  directory for the binary; default: ~/.local/bin
#   DOMUX_HOOKS        no: do not install the agent hooks; default: install them for every
#                      agent whose configuration directory is already there
#   DOMUX_STAY_AWAKE   yes or no: answer the lid question without asking; default: ask when a
#                      terminal is there, and say what to run when it is not
#   NO_COLOR           set to anything to print the logo and progress without color
#
# Output: progress and errors on stderr; on success one line on stdout: "<installed path> <version>".
# Exit status: 0 installed, 1 failed; every failure names the state and the next action.
# Network: three requests, to api.github.com and github.com: the release list, the archive,
# SHA256SUMS.

set -eu

REPO="pranav7/domux"
INSTALL_DIR="${DOMUX_INSTALL_DIR:-$HOME/.local/bin}"
RELEASES="https://github.com/$REPO/releases"
API="https://api.github.com/repos/$REPO/releases?per_page=30"
SOURCE="https://github.com/$REPO#build-from-source"
ISSUES="https://github.com/$REPO/issues"

# Mauve, the color domux draws its own logo in, and the two ends of the band that runs along it.
# Color goes to a terminal only, so a log file or a pipe gets plain text.
if [ -t 2 ] && [ -z "${NO_COLOR:-}" ]; then
  MAUVE=$(printf '\033[38;2;203;166;247m')
  MID=$(printf '\033[38;2;223;199;250m')
  BRIGHT=$(printf '\033[38;2;245;235;255m')
  DIM=$(printf '\033[2m')
  OFF=$(printf '\033[0m')
  EOL=$(printf '\033[K')
else
  MAUVE=""
  MID=""
  BRIGHT=""
  DIM=""
  OFF=""
  EOL=""
fi

# The five letters, each as its top and bottom half. Splitting the logo by letter is what lets
# the band move along it the way it moves along a working agent's word.
# An underscore stands in for a space inside a letter, so the list splits on spaces between
# letters and nowhere else; logo_row puts the spaces back.
LETTERS='█▀▄|█▄▀ █▀█|█▄█ █▀▄▀█|█_▀_█ █_█|█▄█ ▀▄▀|█_█'

# logo_row <top|bottom> <letter the band sits on, or 0 for none>
logo_row() {
  row=$1
  peak=$2
  i=0
  out=""
  for pair in $LETTERS; do
    i=$((i + 1))
    if [ "$row" = top ]; then
      glyph=$(printf '%s' "${pair%%|*}" | tr '_' ' ')
    else
      glyph=$(printf '%s' "${pair##*|}" | tr '_' ' ')
    fi
    d=$((i - peak))
    if [ "$d" -lt 0 ]; then d=$((0 - d)); fi
    if [ "$peak" = 0 ]; then
      color=$MAUVE
    elif [ "$d" = 0 ]; then
      color=$BRIGHT
    elif [ "$d" = 1 ]; then
      color=$MID
    else
      color=$MAUVE
    fi
    out="$out$color$glyph$OFF "
  done
  printf '%s' "$out"
}

# logo_frame <letter the band sits on, or 0 for none>
logo_frame() {
  printf '  %s domux installer%s\n' "$(logo_row top "$1")" "$EOL" >&2
  printf '  %s %sgithub.com/%s%s%s\n' "$(logo_row bottom "$1")" "$DIM" "$REPO" "$OFF" "$EOL" >&2
}

logo() {
  printf '\n' >&2
  logo_frame 0
  # One pass of the band, out and back, on the 70 ms tick the agent rows use. A terminal only:
  # redrawing in place means nothing to a file, and the static logo above is already there.
  if [ -t 2 ] && [ -n "$MAUVE" ]; then
    for peak in 1 2 3 4 5 4 3 2 1; do
      printf '\033[2A' >&2
      logo_frame "$peak"
      sleep 0.07 2>/dev/null || true
    done
    printf '\033[2A' >&2
    logo_frame 0
  fi
  printf '\n' >&2
}

say() {
  printf '  %s>%s %s\n' "$MAUVE" "$OFF" "$*" >&2
}

fail() {
  # fail <state> <next action>
  printf '\ndomux install: %s\n  next: %s\n\n' "$1" "$2" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is not installed" "install $1 with your package manager, then run this script again"
}

logo

need uname
need curl
need tar
need mktemp

if command -v sha256sum >/dev/null 2>&1; then
  checksum() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  fail "neither sha256sum nor shasum is installed" "install coreutils (Linux) or perl (macOS), then run this script again"
fi

# --- platform -------------------------------------------------------------------------------

os_raw=$(uname -s)
case $os_raw in
  Darwin) os=darwin; os_say=macos ;;
  Linux)  os=linux;  os_say=linux ;;
  *) fail "domux runs on macOS and Linux; this system is $os_raw" "build from source: $SOURCE" ;;
esac

arch_raw=$(uname -m)
case $arch_raw in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64)  arch=amd64 ;;
  *) fail "no release build for the $arch_raw architecture; releases cover arm64 and x86_64" "build from source: $SOURCE" ;;
esac

if [ "$os" = linux ] && command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
  fail "release builds link glibc; this system uses musl" "build from source: $SOURCE"
fi

say "detected $os_say/$arch"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

# --- version --------------------------------------------------------------------------------

version="${DOMUX_VERSION:-}"
case $version in
  "") ;;
  [0-9]*) version="v$version" ;;
esac

if [ -z "$version" ]; then
  say "finding the newest release..."
  curl -fsSL -o "$tmp/releases.json" "$API" \
    || fail "could not list releases at $API" "check the network (GitHub allows 60 unauthenticated API requests an hour), or set DOMUX_VERSION=v1.x.y to skip the lookup"
  # v0 tags are domux V1, the Go version, and are not what this script installs.
  tags=$(grep -o '"tag_name": *"[^"]*"' "$tmp/releases.json" | sed 's/.*"\(v[^"]*\)"$/\1/' | grep '^v[1-9]' || true)
  [ -n "$tags" ] || fail "no domux release exists yet at $RELEASES" "set DOMUX_VERSION=v1.x.y once one is published, or build from source: $SOURCE"
  stable=$(printf '%s\n' "$tags" | grep -v -e '-' | head -n 1 || true)
  version=${stable:-$(printf '%s\n' "$tags" | head -n 1)}
fi

case $version in
  v0.*) fail "release $version is domux V1, the Go version; this script installs the Rust version" "set DOMUX_VERSION to a v1 release or later from $RELEASES" ;;
  v[1-9]*) ;;
  *) fail "DOMUX_VERSION must be a release tag such as v1.0.0; got \"$version\"" "set DOMUX_VERSION=v1.x.y and run this script again" ;;
esac
body=${version#v}
archive="domux_${body}_${os}_${arch}.tar.gz"
base="$RELEASES/download/$version"

# --- download and verify --------------------------------------------------------------------

say "downloading $version..."
curl -fsSL -o "$tmp/$archive" "$base/$archive" \
  || fail "could not download $base/$archive" "check that $version has a ${os}_${arch} build at $RELEASES/tag/$version, or set DOMUX_VERSION to another release"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" \
  || fail "could not download $base/SHA256SUMS" "the release is incomplete; try again later or set DOMUX_VERSION to another release"

want=$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$want" ] || fail "SHA256SUMS for $version has no entry for $archive" "the release is incomplete; report it at $ISSUES"
got=$(checksum "$tmp/$archive")
[ "$got" = "$want" ] || fail "checksum mismatch for $archive: want $want, got $got" "do not run the downloaded file; run this script again, and if it repeats report it at $ISSUES"
say "checksum verified"

# --- install --------------------------------------------------------------------------------

mkdir "$tmp/x"
tar -xzf "$tmp/$archive" -C "$tmp/x"
[ -f "$tmp/x/domux" ] || fail "$archive has no domux binary inside" "report it at $ISSUES"
mkdir -p "$INSTALL_DIR" 2>/dev/null || fail "could not create $INSTALL_DIR" "set DOMUX_INSTALL_DIR to a directory you can write to"
[ -w "$INSTALL_DIR" ] || fail "cannot write to $INSTALL_DIR" "set DOMUX_INSTALL_DIR to a directory you can write to"
chmod 0755 "$tmp/x/domux"
# The rename is atomic, so a running domux keeps its own inode and a failed copy never leaves a
# half-written binary on PATH.
mv -f "$tmp/x/domux" "$INSTALL_DIR/domux.tmp"
mv -f "$INSTALL_DIR/domux.tmp" "$INSTALL_DIR/domux"

if ! installed=$("$INSTALL_DIR/domux" --version </dev/null 2>"$tmp/version.err"); then
  [ -s "$tmp/version.err" ] && say "$(cat "$tmp/version.err")"
  fail "$INSTALL_DIR/domux was installed but does not run on this system" "on Linux, domux needs glibc 2.35 or newer (ldd --version shows yours); on macOS, see the README's requirements; otherwise report the message above at $ISSUES"
fi

say "installed $installed to $INSTALL_DIR/domux"
printf '%s %s\n' "$INSTALL_DIR/domux" "$body"

domux="$INSTALL_DIR/domux"

# --- hooks --------------------------------------------------------------------------------

# An agent reports its state through a hook, so the hooks are the difference between a row that
# says what an agent is doing and a row that says unknown. They go in only for an agent whose
# configuration directory is already there: installing them for an agent the reader does not run
# would create a configuration file for a tool they never asked for.
hooks_dir() {
  case $1 in
    claude)   printf '%s\n' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}" ;;
    codex)    printf '%s\n' "$HOME/.codex" ;;
    opencode) printf '%s\n' "$HOME/.config/opencode" ;;
  esac
}

if [ "${DOMUX_HOOKS:-}" = no ]; then
  say "skipped the agent hooks; run '$INSTALL_DIR/domux install claude --apply' to add them"
else
  found=""
  for kind in claude codex opencode; do
    dir=$(hooks_dir "$kind")
    [ -d "$dir" ] || continue
    found="$found $kind"
    if "$domux" install "$kind" --apply >"$tmp/hooks.out" 2>&1; then
      say "hooks installed for $kind"
    else
      say "could not install the $kind hooks: $(tr '\n' ' ' < "$tmp/hooks.out")"
      say "run '$INSTALL_DIR/domux install $kind --apply' to see what happened"
    fi
  done
  if [ -z "$found" ]; then
    say "no agent is configured on this machine yet, so no hooks were installed"
    say "run '$INSTALL_DIR/domux install claude --apply' once Claude Code, Codex or OpenCode is there"
  fi
fi

# --- the lid ------------------------------------------------------------------------------

# Stay awake holds the machine awake while agents work. It needs nothing installed anywhere
# except on macOS, where covering a closed lid takes a launch daemon and a sudoers line, and
# that is the one thing in domux that asks for sudo.
# True when the terminal can actually be opened. The file can be there and still refuse to open,
# in a session with no controlling terminal, so this opens it rather than asking about the path.
tty_is_open() {
  (: < /dev/tty) 2>/dev/null
}

stay_awake_setup() {
  printf '\n' >&2
  # sudo asks for a password on the terminal, which is not this script's stdin under
  # "curl | sh", so the command reads the terminal directly when there is one.
  lid_code=0
  if tty_is_open; then
    "$domux" stay-awake install --full --apply </dev/tty >"$tmp/lid.out" 2>&1 || lid_code=$?
  else
    "$domux" stay-awake install --full --apply >"$tmp/lid.out" 2>&1 || lid_code=$?
  fi
  if [ "$lid_code" = 0 ]; then
    say "full stay awake is set up"
    say "add mode = \"full\" under [stay_awake] in ~/.config/domux/domux.toml to turn it on"
  else
    say "could not set up full stay awake: $(tr '\n' ' ' < "$tmp/lid.out")"
    say "domux still holds the machine awake while the lid is open"
  fi
}

if [ "$os" = darwin ]; then
  printf '\n' >&2
  case "${DOMUX_STAY_AWAKE:-}" in
    yes) stay_awake_setup ;;
    no)  say "skipped the lid setup; run '$INSTALL_DIR/domux stay-awake install --full --apply' later" ;;
    *)
      if [ -t 2 ] && tty_is_open; then
        say "domux can stop a closed lid putting this machine to sleep while agents work."
        say "it asks for sudo once, to write a launch daemon and a sudoers line."
        printf '  %s>%s set that up now? [y/N] ' "$MAUVE" "$OFF" >&2
        answer=""
        read -r answer < /dev/tty || answer=""
        case $answer in
          y|Y|yes|Yes) stay_awake_setup ;;
          *) say "skipped. run '$INSTALL_DIR/domux stay-awake install --full --apply' to set it up later" ;;
        esac
      else
        say "to stop a closed lid putting this machine to sleep, run:"
        say "  $INSTALL_DIR/domux stay-awake install --full --apply"
      fi
      ;;
  esac
fi

# --- what to do next ------------------------------------------------------------------------

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    printf '\n' >&2
    say "$INSTALL_DIR is not on your PATH. Add this line to ~/.zshrc or ~/.bashrc and open a new shell:"
    # shellcheck disable=SC2016  # the line is printed for the reader to paste, so $PATH is literal
    printf '\n      export PATH="%s:$PATH"\n\n' "$INSTALL_DIR" >&2
    say "fish: fish_add_path $INSTALL_DIR"
    ;;
esac

printf '\n' >&2
say "ready. run 'domux' to get started."
printf '\n' >&2
