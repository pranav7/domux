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
#   NO_COLOR           set to anything to print without color or animation
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

# --- the look ---------------------------------------------------------------------------------
#
# The installer is drawn the way domux draws itself: mauve, a glyph that turns while something
# is happening, and a band that runs along the word beside it. A step that is done settles to
# the glyph's own resting frame, a dim dot. Everything here is a terminal's: a pipe or a log
# file gets the same lines with no color, no redrawing and no animation.

if [ -t 2 ] && [ -z "${NO_COLOR:-}" ]; then
  MAUVE=$(printf '\033[38;2;203;166;247m')      # the domux mauve
  MID=$(printf '\033[38;2;223;199;250m')        # one step along the band
  BRIGHT=$(printf '\033[38;2;245;235;255m')     # the band's peak
  FAINT=$(printf '\033[38;2;122;103;148m')      # a step that is done
  RED=$(printf '\033[38;2;243;139;168m')        # the dot, which means domux is waiting on you
  OFF=$(printf '\033[0m')
  EOL=$(printf '\033[K')
  LIVE=yes
else
  MAUVE=""; MID=""; BRIGHT=""; FAINT=""; RED=""; OFF=""; EOL=""
  LIVE=""
fi

# domux's own glyph: sparse to dense and back, so each frame morphs into the next. It turns
# every second tick while the band moves on every one, which is the ratio the agent rows use.
GLYPHS='· ✦ ✶ ✳ ✢ ✻ ✽ ✻ ✢ ✳ ✶ ✦ ·'
TICK=0

# set_glyph <tick>: GLYPH becomes the frame for that tick.
set_glyph() {
  g_want=$((($1 / 2) % 13))
  g_i=0
  for g_frame in $GLYPHS; do
    if [ "$g_i" = "$g_want" ]; then
      GLYPH=$g_frame
      return 0
    fi
    g_i=$((g_i + 1))
  done
}

# set_band <word> <tick>: BAND becomes the word with the bright band where it has reached. The
# peak travels from before the first character to past the last and back, so the band enters and
# leaves the word rather than appearing on it.
set_band() {
  b_word=$1
  b_span=$((${#b_word} + 6))
  b_pos=$(($2 % (b_span * 2)))
  if [ "$b_pos" -ge "$b_span" ]; then b_pos=$((b_span * 2 - b_pos)); fi
  b_peak=$((b_pos - 3))
  b_rest=$b_word
  b_out=""
  b_i=0
  while [ -n "$b_rest" ]; do
    b_head=${b_rest#?}
    b_ch=${b_rest%"$b_head"}
    b_rest=$b_head
    b_i=$((b_i + 1))
    b_d=$((b_i - b_peak))
    if [ "$b_d" -lt 0 ]; then b_d=$((0 - b_d)); fi
    if [ "$b_d" = 0 ]; then
      b_c=$BRIGHT
    elif [ "$b_d" = 1 ]; then
      b_c=$MID
    else
      b_c=$MAUVE
    fi
    b_out="$b_out$b_c$b_ch"
  done
  BAND="$b_out$OFF"
}

# The five letters of the logo, each as its top and bottom half, so the band can run along the
# logo the way it runs along a word. An underscore stands in for a space inside a letter, so the
# list splits between letters and nowhere else.
LETTERS='█▀▄|█▄▀ █▀█|█▄█ █▀▄▀█|█_▀_█ █_█|█▄█ ▀▄▀|█_█'

# set_logo_row <top|bottom> <letter the band sits on, or 0 for none>
set_logo_row() {
  l_row=$1
  l_peak=$2
  l_i=0
  l_out=""
  for l_pair in $LETTERS; do
    l_i=$((l_i + 1))
    if [ "$l_row" = top ]; then
      l_glyph=${l_pair%%|*}
    else
      l_glyph=${l_pair##*|}
    fi
    l_glyph=$(printf '%s' "$l_glyph" | tr '_' ' ')
    l_d=$((l_i - l_peak))
    if [ "$l_d" -lt 0 ]; then l_d=$((0 - l_d)); fi
    if [ "$l_peak" = 0 ]; then
      l_c=$MAUVE
    elif [ "$l_d" = 0 ]; then
      l_c=$BRIGHT
    elif [ "$l_d" = 1 ]; then
      l_c=$MID
    else
      l_c=$MAUVE
    fi
    l_out="$l_out$l_c$l_glyph$OFF "
  done
  LOGO_ROW=$l_out
}

logo_frame() {
  set_logo_row top "$1"
  printf '   %s%s\n' "$LOGO_ROW" "$EOL" >&2
  set_logo_row bottom "$1"
  printf '   %s %s%s%s%s\n' "$LOGO_ROW" "$FAINT" "github.com/$REPO" "$OFF" "$EOL" >&2
}

logo() {
  printf '\n' >&2
  logo_frame 0
  if [ -n "$LIVE" ]; then
    for l_peak in 1 2 3 4 5 4 3 2 1 0; do
      printf '\033[2A' >&2
      logo_frame "$l_peak"
      sleep 0.07 2>/dev/null || true
    done
  fi
  printf '\n' >&2
}

# step <word>: the line that is redrawn while something happens. Only ever on a terminal.
step() {
  set_glyph "$TICK"
  set_band "$1" "$TICK"
  printf '\r   %s%s%s  %s%s' "$MAUVE" "$GLYPH" "$OFF" "$BAND" "$EOL" >&2
}

# working <word> <command...>: runs the command while the step turns, and answers its status.
# Without a terminal the command runs on its own and nothing is drawn.
working() {
  w_word=$1
  shift
  if [ -z "$LIVE" ]; then
    "$@"
    return $?
  fi
  "$@" &
  w_pid=$!
  while kill -0 "$w_pid" 2>/dev/null; do
    step "$w_word"
    TICK=$((TICK + 1))
    sleep 0.07 2>/dev/null || true
  done
  w_code=0
  wait "$w_pid" || w_code=$?
  printf '\r%s' "$EOL" >&2
  return $w_code
}

# done_line <text>: a step that is finished, marked with the glyph's resting frame.
done_line() {
  printf '   %s·%s  %s\n' "$FAINT" "$OFF" "$*" >&2
}

# note <text>: something the reader should see that is not a step.
note() {
  printf '      %s%s%s\n' "$FAINT" "$*" "$OFF" >&2
}

# value <text>: a value inside a line, in the mauve.
value() {
  printf '%s%s%s' "$MAUVE" "$1" "$OFF"
}

fail() {
  # fail <state> <next action>. The two lines keep the shape every domux failure has, the state
  # and then the next action; only the prefix is colored.
  printf '\r%s' "$EOL" >&2
  printf '\n%sdomux install:%s %s\n  next: %s\n\n' "$RED" "$OFF" "$1" "$2" >&2
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

done_line "$(value "$os_say") on $(value "$arch")"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

# --- version --------------------------------------------------------------------------------

version="${DOMUX_VERSION:-}"
case $version in
  "") ;;
  [0-9]*) version="v$version" ;;
esac

if [ -z "$version" ]; then
  working Looking curl -fsSL -o "$tmp/releases.json" "$API" \
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

done_line "release $(value "$version")"

# --- download and verify --------------------------------------------------------------------

working Downloading curl -fsSL -o "$tmp/$archive" "$base/$archive" \
  || fail "could not download $base/$archive" "check that $version has a ${os}_${arch} build at $RELEASES/tag/$version, or set DOMUX_VERSION to another release"
working Downloading curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" \
  || fail "could not download $base/SHA256SUMS" "the release is incomplete; try again later or set DOMUX_VERSION to another release"

want=$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$want" ] || fail "SHA256SUMS for $version has no entry for $archive" "the release is incomplete; report it at $ISSUES"
got=$(checksum "$tmp/$archive")
[ "$got" = "$want" ] || fail "checksum mismatch for $archive: want $want, got $got" "do not run the downloaded file; run this script again, and if it repeats report it at $ISSUES"

bytes=$(wc -c < "$tmp/$archive" | tr -d ' ')
size=$(awk -v b="$bytes" 'BEGIN {
  if (b >= 1048576) printf "%.1f MB", b / 1048576
  else if (b >= 1024) printf "%.0f KB", b / 1024
  else printf "%d bytes", b
}' 2>/dev/null || echo "")
if [ -n "$size" ]; then
  done_line "$(value "$size") downloaded, checksum verified"
else
  done_line "checksum verified"
fi

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
  [ -s "$tmp/version.err" ] && note "$(cat "$tmp/version.err")"
  fail "$INSTALL_DIR/domux was installed but does not run on this system" "on Linux, domux needs glibc 2.35 or newer (ldd --version shows yours); on macOS, see the README's requirements; otherwise report the message above at $ISSUES"
fi

done_line "$(value "$installed") installed to $(value "$INSTALL_DIR/domux")"
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
  done_line "hooks skipped"
  note "run $INSTALL_DIR/domux install claude --apply to add them"
else
  found=""
  for kind in claude codex opencode; do
    dir=$(hooks_dir "$kind")
    [ -d "$dir" ] || continue
    if working Wiring "$domux" install "$kind" --apply >"$tmp/hooks.out" 2>&1; then
      found="$found $kind"
    else
      done_line "could not install the $kind hooks: $(tr '\n' ' ' < "$tmp/hooks.out")"
      note "run $INSTALL_DIR/domux install $kind --apply to see what happened"
    fi
  done
  if [ -n "$found" ]; then
    done_line "hooks for$(value "$found")"
  else
    done_line "no agent is configured on this machine yet, so no hooks were installed"
    note "run $INSTALL_DIR/domux install claude --apply once Claude Code, Codex or OpenCode is there"
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

# ask <question>: the red dot means domux is waiting on you, the same as an agent row. Answers 0
# for yes. One keypress when the terminal allows it, a typed line when it does not.
ask() {
  printf '\n   %s◉%s  %s\n' "$RED" "$OFF" "$1" >&2
  printf '      %sy%s or %sn%s  ' "$MAUVE" "$OFF" "$MAUVE" "$OFF" >&2
  a_answer=""
  a_saved=$(stty -g < /dev/tty 2>/dev/null || echo "")
  if [ -n "$a_saved" ] && stty -icanon -echo min 1 time 0 < /dev/tty 2>/dev/null; then
    a_answer=$(dd bs=1 count=1 2>/dev/null < /dev/tty || echo "")
    stty "$a_saved" < /dev/tty 2>/dev/null || true
    printf '%s\n\n' "$a_answer" >&2
  else
    read -r a_answer < /dev/tty || a_answer=""
    printf '\n' >&2
  fi
  case $a_answer in
    y|Y) return 0 ;;
    *) return 1 ;;
  esac
}

stay_awake_setup() {
  # sudo asks for a password on the terminal, which is not this script's stdin under
  # "curl | sh", so the command reads the terminal directly when there is one.
  lid_code=0
  if tty_is_open; then
    "$domux" stay-awake install --full --apply </dev/tty >"$tmp/lid.out" 2>&1 || lid_code=$?
  else
    "$domux" stay-awake install --full --apply >"$tmp/lid.out" 2>&1 || lid_code=$?
  fi
  if [ "$lid_code" = 0 ]; then
    done_line "the lid is covered too"
    note "add mode = \"full\" under [stay_awake] in ~/.config/domux/domux.toml to turn it on"
  else
    done_line "could not set up full stay awake: $(tr '\n' ' ' < "$tmp/lid.out")"
    note "domux still holds the machine awake while the lid is open"
  fi
}

if [ "$os" = darwin ]; then
  case "${DOMUX_STAY_AWAKE:-}" in
    yes) stay_awake_setup ;;
    no)  done_line "the lid setup was skipped"
         note "run $INSTALL_DIR/domux stay-awake install --full --apply later" ;;
    *)
      if [ -n "$LIVE" ] && tty_is_open; then
        if ask "Stop a closed lid putting this machine to sleep? It asks for sudo once."; then
          stay_awake_setup
        else
          done_line "the lid setup was skipped"
          note "run $INSTALL_DIR/domux stay-awake install --full --apply later"
        fi
      else
        done_line "the lid needs one more command"
        note "run $INSTALL_DIR/domux stay-awake install --full --apply"
      fi
      ;;
  esac
fi

# --- what to do next ------------------------------------------------------------------------

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    printf '\n   %s◉%s  %s is not on your PATH\n' "$RED" "$OFF" "$INSTALL_DIR" >&2
    # shellcheck disable=SC2016  # the line is printed for the reader to paste, so $PATH is literal
    printf '      %sexport PATH="%s:$PATH"%s   in ~/.zshrc or ~/.bashrc\n' "$MAUVE" "$INSTALL_DIR" "$OFF" >&2
    note "fish: fish_add_path $INSTALL_DIR"
    ;;
esac

printf '\n   %sdomux is ready.%s  run %sdomux%s\n\n' "$BRIGHT" "$OFF" "$MAUVE" "$OFF" >&2
