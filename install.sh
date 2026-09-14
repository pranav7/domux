#!/bin/sh
# domux installer. Downloads the release archive for this machine, verifies its checksum,
# installs the domux binary and the agent hooks, and asks for the leader and for stay awake. No
# shell startup file is edited.
#
#   curl -fsSL https://domux.dev/install.sh | sh
#
# Environment:
#   DOMUX_VERSION      release tag to install, for example v0.1.1; default: the newest release
#                      (stable first; a prerelease only when no stable release exists)
#   DOMUX_INSTALL_DIR  directory for the binary; default: ~/.local/bin
#   DOMUX_CONFIG_FILE  the config file the leader and stay awake go in; default:
#                      ~/.config/domux/domux.toml, the file domux itself reads
#   DOMUX_HOOKS        no: do not install the agent hooks; default: install them for every
#                      agent whose configuration directory is already there
#   DOMUX_LEADER       a key name such as C-a: answer the leader question without asking;
#                      default: ask when a terminal is there, and C-s when it is not
#   DOMUX_STAY_AWAKE   yes or no: answer the stay awake question without asking; default: ask
#                      when a terminal is there, and no when it is not
#   NO_COLOR           set to anything to print without color or animation
#
# A leader or a stay awake mode the config file already sets is kept, whatever the answer.
#
# Output: progress and errors on stderr; on success one line on stdout when stdout is not a
# terminal: "<installed path> <version>".
# Writes: the binary, the agent hooks, and in the config file only the lines it does not have.
# Exit status: 0 installed, 1 failed; every failure names the state and the next action.
# Network: three requests, to api.github.com and github.com: the release list, the archive,
# SHA256SUMS.

set -eu

REPO="pranav7/domux"
INSTALL_DIR="${DOMUX_INSTALL_DIR:-$HOME/.local/bin}"
CONFIG_FILE="${DOMUX_CONFIG_FILE:-$HOME/.config/domux/domux.toml}"
RELEASES="https://github.com/$REPO/releases"
API="https://api.github.com/repos/$REPO/releases?per_page=30"
SOURCE="https://github.com/$REPO"
ISSUES="https://github.com/$REPO/issues"
LEADER_DEFAULT="C-s"

# --- the look ---------------------------------------------------------------------------------
#
# The logo, a check mark on a step that is done, a cross on a step that did not work while the
# install carries on, the red dot when domux is waiting on you, and a spinner beside a step that
# waits on the network. Only the spinner moves, and only there: every other step is over before a
# frame could be read, so it prints its check mark at once. A pipe, a log file and NO_COLOR get
# the same lines once, in order, with no color and no redrawing.

if [ -t 2 ] && [ -z "${NO_COLOR:-}" ]; then
  MAUVE=$(printf '\033[38;2;203;166;247m')      # the domux mauve
  BRIGHT=$(printf '\033[38;2;245;235;255m')     # the last line, once domux is installed
  FAINT=$(printf '\033[38;2;122;103;148m')      # a note under a step
  RED=$(printf '\033[38;2;243;139;168m')        # the dot, which means domux is waiting on you
  OFF=$(printf '\033[0m')
  EOL=$(printf '\033[K')
  CR=$(printf '\r')
  HIDE=$(printf '\033[?25l')
  SHOW=$(printf '\033[?25h')
  LIVE=yes
else
  MAUVE=""; BRIGHT=""; FAINT=""; RED=""; OFF=""; EOL=""; CR=""; HIDE=""; SHOW=""
  LIVE=""
fi

logo() {
  printf '\n   %s█▀▄ █▀█ █▀▄▀█ █ █ ▀▄▀%s\n' "$MAUVE" "$OFF" >&2
  printf '   %s█▄▀ █▄█ █ ▀ █ █▄█ █ █%s  %s%s%s\n\n' \
    "$MAUVE" "$OFF" "$FAINT" "github.com/$REPO" "$OFF" >&2
}

# done_line <text>: a step that is finished, marked with a check mark. On a terminal it replaces
# the spinner's last frame when one is still on the line.
done_line() {
  printf '%s   %s✓%s  %s%s\n' "$CR" "$MAUVE" "$OFF" "$*" "$EOL" >&2
}

# failed_line <text>: a step that did not work, when the install carries on without it.
failed_line() {
  printf '%s   %s✗%s  %s%s\n' "$CR" "$RED" "$OFF" "$*" "$EOL" >&2
}

# note <text>: something the reader should see under a step.
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
  printf '%s%s' "$CR" "$EOL" >&2
  printf '\n%sdomux install:%s %s\n  next: %s\n\n' "$RED" "$OFF" "$1" "$2" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is not installed" "install $1 with your package manager, then run this script again"
}

# --- the spinner ------------------------------------------------------------------------------

SPIN_PID=""
FRAMES_DRAWN=0

# frame: FRAME becomes the next braille frame, from the set every command line spinner uses.
frame() {
  case $((FRAMES_DRAWN % 10)) in
    0) FRAME='⠋' ;; 1) FRAME='⠙' ;; 2) FRAME='⠹' ;; 3) FRAME='⠸' ;; 4) FRAME='⠼' ;;
    5) FRAME='⠴' ;; 6) FRAME='⠦' ;; 7) FRAME='⠧' ;; 8) FRAME='⠇' ;; *) FRAME='⠏' ;;
  esac
}

# spin <word> <command...>: runs a command that waits on the network and answers its status. On
# a terminal the command runs in the background while a frame turns beside the word. The script
# looks every 40 ms, so a request costs no more time than it takes. The first frame waits for the
# sixth look, a quarter second, so a request that answers in less time than a frame can be read
# draws nothing, and then a frame turns every second look, every 80 ms. The last frame stays
# until the step's own line replaces it, so two requests in a row read as one step. What the
# command writes on stderr is shown only when it fails.
spin() {
  s_word=$1
  shift
  s_code=0
  if [ -z "$LIVE" ]; then
    "$@" </dev/null >/dev/null 2>"$tmp/spin.err" || s_code=$?
  else
    "$@" </dev/null >/dev/null 2>"$tmp/spin.err" &
    SPIN_PID=$!
    printf '%s' "$HIDE" >&2
    s_look=0
    s_next=6
    while :; do
      # A sleep that takes no fraction sleeps a second, and every look after it draws a frame.
      sleep 0.04 2>/dev/null || { sleep 1; s_look=$s_next; }
      s_look=$((s_look + 1))
      kill -0 "$SPIN_PID" 2>/dev/null || break
      [ "$s_look" -ge "$s_next" ] || continue
      s_next=$((s_look + 2))
      frame
      printf '%s   %s%s%s  %s%s' "$CR" "$MAUVE" "$FRAME" "$OFF" "$s_word" "$EOL" >&2
      FRAMES_DRAWN=$((FRAMES_DRAWN + 1))
    done
    wait "$SPIN_PID" || s_code=$?
    SPIN_PID=""
    [ "$s_code" = 0 ] || printf '%s%s' "$CR" "$EOL" >&2
    printf '%s' "$SHOW" >&2
  fi
  if [ "$s_code" != 0 ] && [ -s "$tmp/spin.err" ]; then
    note "$(tr '\n' ' ' < "$tmp/spin.err")"
  fi
  return "$s_code"
}

# --- the terminal -----------------------------------------------------------------------------

TTY_SAVED=""     # the terminal's settings from before the first question, which cleanup gives back
TTY_ASKING=""    # the same settings with flow control off, kept from the first question to the exit
KEY_WAITING=""   # yes while a question waits on the reader, so an interrupt ends the prompt's line

# True when the terminal can actually be opened. The file can be there and still refuse to open,
# in a session with no controlling terminal, so this opens it rather than asking about the path.
tty_is_open() {
  (: < /dev/tty) 2>/dev/null
}

# asking: true when there is a reader to ask. A question goes where the steps go, so stderr has
# to be the terminal too: a log file would get the question and nobody would see it.
asking() {
  [ -t 2 ] && tty_is_open
}

# flow_control_off: turns flow control off on the terminal and keeps it off until the script
# exits, when cleanup gives back the settings from before. C-s is XOFF: with flow control on, the
# terminal keeps the key and stops all output until C-q. A reader who presses the leader at its
# own question can press it twice, or hold it, and a second C-s can land after one question and
# before the next, so turning flow control back on after each key would still stop the install.
# Fails when the terminal's settings cannot be read or changed.
flow_control_off() {
  if [ -z "$TTY_ASKING" ]; then
    [ -n "$TTY_SAVED" ] || TTY_SAVED=$(stty -g < /dev/tty 2>/dev/null) || TTY_SAVED=""
    [ -n "$TTY_SAVED" ] || return 1
    stty -ixon < /dev/tty 2>/dev/null || return 1
    TTY_ASKING=$(stty -g < /dev/tty 2>/dev/null) || TTY_ASKING=""
  fi
  [ -n "$TTY_ASKING" ]
}

# read_key: one keypress from the terminal into KEY, with echo off while it waits. KEY is empty
# for enter, the digit or the letter for 1 to 5, y and n, the key name for C-s, C-a, C-b and
# C-Space, and "other" for any other key. Keys typed before the question are thrown away, so a
# key pressed while a download turned is not taken as the answer, and so is the rest of a key
# that sends more than one byte, such as an arrow. Where the terminal will not give single keys,
# a typed line instead, and KEY_LINE is yes.
read_key() {
  KEY=""
  KEY_LINE=""
  if flow_control_off && command -v od >/dev/null 2>&1 \
    && stty -icanon -echo min 0 time 0 < /dev/tty 2>/dev/null; then
    KEY_WAITING=yes
    dd bs=1024 count=1 < /dev/tty >/dev/null 2>&1 || true
    stty min 1 time 0 < /dev/tty 2>/dev/null || true
    # od writes the byte as hex, so C-Space, which sends a zero byte, survives the shell.
    k_byte=$(dd bs=1 count=1 < /dev/tty 2>/dev/null | od -An -tx1 | tr -d ' \n')
    stty "$TTY_ASKING" < /dev/tty 2>/dev/null || true
    KEY_WAITING=""
    case $k_byte in
      ""|0a|0d) KEY="" ;;
      00) KEY=C-Space ;;
      01) KEY=C-a ;;
      02) KEY=C-b ;;
      13) KEY=C-s ;;
      3[1-5]) KEY=${k_byte#3} ;;
      59|79) KEY=y ;;
      4e|6e) KEY=n ;;
      *) KEY=other ;;
    esac
  else
    KEY_LINE=yes
    KEY_WAITING=yes
    read -r KEY < /dev/tty || KEY=""
    KEY_WAITING=""
  fi
}

# ask <question> [note]: the red dot means domux is waiting on you, the same as an agent row.
# The note, when there is one, goes under the question. Enter answers no, and a key that is
# neither y nor n is not an answer. Answers 0 for yes.
ask() {
  printf '\n   %s◉%s  %s\n' "$RED" "$OFF" "$1" >&2
  [ -z "${2:-}" ] || note "$2"
  printf '      %sy%s or %sn%s  ' "$MAUVE" "$OFF" "$MAUVE" "$OFF" >&2
  while :; do
    read_key
    case $KEY in
      y|Y) a_yes=y ;;
      ""|n|N) a_yes="" ;;
      *) [ -n "$KEY_LINE" ] || continue; a_yes="" ;;
    esac
    break
  done
  if [ -n "$KEY_LINE" ]; then
    printf '\n' >&2
  else
    printf '%s\n\n' "${a_yes:-n}" >&2
  fi
  [ -n "$a_yes" ]
}

# cleanup: runs on every exit. It stops the request spin started, and nothing else, gives the
# terminal back its cursor and its settings, and removes the temp dir. A write to a terminal that
# has closed fails, so no step here stops the ones after it.
cleanup() {
  if [ -n "$SPIN_PID" ]; then
    kill "$SPIN_PID" 2>/dev/null || true
    wait "$SPIN_PID" 2>/dev/null || true
    SPIN_PID=""
    printf '%s%s%s' "$CR" "$EOL" "$SHOW" >&2 2>/dev/null || true
  fi
  if [ -n "$TTY_SAVED" ]; then
    stty "$TTY_SAVED" < /dev/tty 2>/dev/null || true
    TTY_SAVED=""
    TTY_ASKING=""
  fi
  if [ -n "$KEY_WAITING" ]; then
    KEY_WAITING=""
    printf '\n' >&2 2>/dev/null || true
  fi
  if [ -n "${tmp:-}" ]; then
    rm -rf "$tmp"
  fi
}

# --- key names --------------------------------------------------------------------------------

# one_character <text>: true when the text is one character that can go in a TOML string, in
# UTF-8, which is the only encoding a TOML file can have. A control character is not one: TOML
# refuses it inside a string, and a byte that is not UTF-8 makes the whole file unreadable, so
# domux would refuse the file either way. The bytes are read in hex because what ? matches
# depends on the shell: dash, and any shell in the C locale, match one byte, so ? would refuse é
# and take a stray byte that is not a character at all.
one_character() {
  if ! command -v od >/dev/null 2>&1; then
    case $1 in ?) return 0 ;; esac
    return 1
  fi
  o_hex=$(printf '%s' "$1" | od -An -tx1 | tr -d ' \n')
  case $o_hex in
    2?|[3456]?|7[0123456789abcde]) return 0 ;;
    c[23456789abcdef][89ab]?|d?[89ab]?) return 0 ;;
    e0[ab]?[89ab]?|e[123456789abcef][89ab]?[89ab]?|ed[89]?[89ab]?) return 0 ;;
    f0[9ab]?[89ab]?[89ab]?|f[123][89ab]?[89ab]?[89ab]?|f48?[89ab]?[89ab]?) return 0 ;;
  esac
  return 1
}

# is_key <name>: true for a key name domux reads under [keys]: any of the modifiers C-, S-, M-
# and D-, then one character, a named key, or F1 to F12.
is_key() {
  k_rest=$1
  while :; do
    case $k_rest in
      [CSMD]-?*) k_rest=${k_rest#??} ;;
      *) break ;;
    esac
  done
  case $k_rest in
    Enter|Tab|Backspace|Esc|Space|Up|Down|Left|Right|Home|End|PageUp|PageDown|Insert|Delete) return 0 ;;
    F[1-9]|F1[0-2]) return 0 ;;
  esac
  one_character "$k_rest"
}

logo

need uname
need curl
need tar
need mktemp
need awk

if command -v sha256sum >/dev/null 2>&1; then
  checksum() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  fail "neither sha256sum nor shasum is installed" "install coreutils (Linux) or perl (macOS), then run this script again"
fi

# Checked before anything is fetched, so a leader domux would refuse costs no download.
if [ -n "${DOMUX_LEADER:-}" ] && ! is_key "$DOMUX_LEADER"; then
  fail "DOMUX_LEADER must be a key name such as C-s, M-a or C-Space; got \"$DOMUX_LEADER\"" "set DOMUX_LEADER to a key name and run this script again"
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

done_line "$(value "$os_say") on $(value "$arch") detected"

tmp=$(mktemp -d)
trap cleanup EXIT
# A terminal that closes sends HUP, and the temp dir and the request go with it the same way.
trap 'cleanup; exit 129' HUP
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

# --- version --------------------------------------------------------------------------------

version="${DOMUX_VERSION:-}"
case $version in
  "") ;;
  [0-9]*) version="v$version" ;;
esac

if [ -z "$version" ]; then
  spin "looking up the newest release" curl -fsSL -o "$tmp/releases.json" "$API" \
    || fail "could not list releases at $API" "check the network (GitHub allows 60 unauthenticated API requests an hour), or set DOMUX_VERSION=v0.x.y to skip the lookup"
  tags=$(grep -o '"tag_name": *"[^"]*"' "$tmp/releases.json" | sed 's/.*"\(v[^"]*\)"$/\1/' | grep '^v[0-9]' || true)
  [ -n "$tags" ] || fail "no domux release exists yet at $RELEASES" "set DOMUX_VERSION=v0.x.y once one is published, or build from source: $SOURCE"
  stable=$(printf '%s\n' "$tags" | grep -v -e '-' | head -n 1 || true)
  version=${stable:-$(printf '%s\n' "$tags" | head -n 1)}
  version_how="found"
else
  version_how="pinned by DOMUX_VERSION"
fi

case $version in
  v[0-9]*) ;;
  *) fail "DOMUX_VERSION must be a release tag such as v0.1.1; got \"$version\"" "set DOMUX_VERSION=v0.x.y and run this script again" ;;
esac
body=${version#v}
archive="domux_${body}_${os}_${arch}.tar.gz"
base="$RELEASES/download/$version"

done_line "release $(value "$version") $version_how"

# --- download and verify --------------------------------------------------------------------

spin "downloading domux $body" curl -fsSL -o "$tmp/$archive" "$base/$archive" \
  || fail "could not download $base/$archive" "check that $version has a ${os}_${arch} build at $RELEASES/tag/$version, or set DOMUX_VERSION to another release"
spin "downloading domux $body" curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" \
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
# A domux already here makes this a reinstall, and that domux may be running.
reinstall=""
[ ! -e "$INSTALL_DIR/domux" ] || reinstall=yes
# The rename is atomic, so a running domux keeps its own inode and a failed copy never leaves a
# half-written binary on PATH.
mv -f "$tmp/x/domux" "$INSTALL_DIR/domux.tmp"
mv -f "$INSTALL_DIR/domux.tmp" "$INSTALL_DIR/domux"

if ! installed=$("$INSTALL_DIR/domux" --version </dev/null 2>"$tmp/version.err"); then
  [ -s "$tmp/version.err" ] && note "$(cat "$tmp/version.err")"
  fail "$INSTALL_DIR/domux was installed but does not run on this system" "on Linux, domux needs glibc 2.35 or newer (ldd --version shows yours); on macOS, see the README's requirements; otherwise report the message above at $ISSUES"
fi

done_line "$(value "$installed") installed to $(value "$INSTALL_DIR/domux")"
# The path and version for a script reading this one's output. A reader gets the line above
# instead, so nothing unstyled lands in the middle of what they are watching.
[ -t 1 ] || printf '%s %s\n' "$INSTALL_DIR/domux" "$body"

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
  note "run $domux install claude --apply to add them"
else
  detected=""
  for kind in claude codex opencode; do
    dir=$(hooks_dir "$kind")
    [ -d "$dir" ] || continue
    detected=yes
    if "$domux" install "$kind" --apply </dev/null >"$tmp/hooks.out" 2>&1; then
      done_line "$(value "$kind") detected, hooks installed"
    else
      failed_line "$(value "$kind") detected, hooks not installed: $(tr '\n' ' ' < "$tmp/hooks.out")"
      note "run $domux install $kind --apply to see what happened"
    fi
  done
  if [ -z "$detected" ]; then
    done_line "no coding agent detected, so no hooks were installed"
    note "run $domux install claude --apply once Claude Code, Codex or OpenCode is there"
  fi
fi

# --- the config file ------------------------------------------------------------------------

# The installer adds a line to the config file and changes nothing else in it, with awk, which
# is on every machine this script runs on and reads the whole file the same way everywhere.
# Decision record 0043 says why it is not sed and not the binary.
#
# classify(line) reads one line of TOML. It sets text to the line without the byte order mark a
# file can start with. It sets header to the table the line opens, "keys" for "[keys]", "["keys"]"
# or "['keys']" with or without a comment after it, or "[keys" for an array of tables,
# "[[keys]]", so that one never matches a table; header is "" for any other line. A quoted part
# of the name can hold any character, so "["ke ys"]" is "ke ys" and never "keys". It sets plain
# when the line starts outside a multi-line array or string, where a key or a header can be, and
# then follows the brackets and multi-line strings the line opens and closes, so a line inside
# one is never read as either.
# shellcheck disable=SC2016  # an awk program: $0 is awk's, not the shell's
CONFIG_AWK_CLASSIFY='
function classify(line,   h, i, n, c, three, bom) {
  bom = "\357\273\277"
  if (NR == 1 && index(line, bom) == 1) line = substr(line, length(bom) + 1)
  text = line
  plain = (depth == 0 && long == "")
  header = ""
  if (plain && line ~ /^[ \t]*\[\[?([A-Za-z0-9_. \t-]|"([^"\\]|\\.)*"|\047[^\047]*\047)+\]\]?[ \t]*(#.*)?\r?$/) {
    h = line
    sub(/^[ \t]*/, "", h)
    header = (substr(h, 1, 2) == "[[") ? "[" : ""
    header = header table_name(h)
    return
  }
  n = length(line)
  for (i = 1; i <= n; i++) {
    c = substr(line, i, 1)
    three = substr(line, i, 3)
    if (long != "") {
      if (three == long) { long = ""; i += 2 }
      else if (c == "\\" && long == "\"\"\"") i++
    } else if (c == "#") {
      break
    } else if (three == "\"\"\"" || three == "\047\047\047") {
      long = three
      i += 2
    } else if (c == "\"") {
      for (i++; i <= n; i++) {
        c = substr(line, i, 1)
        if (c == "\\") i++
        else if (c == "\"") break
      }
    } else if (c == "\047") {
      i++
      while (i <= n && substr(line, i, 1) != "\047") i++
    } else if (c == "[") {
      depth++
    } else if (c == "]") {
      depth--
    }
  }
}
# table_name(h): the name in a table header, without the blanks and quotes around its parts. A
# quoted part keeps every character in it, and a ] inside one does not end the name.
function table_name(h,   i, n, c, q, name) {
  sub(/^\[\[?/, "", h)
  n = length(h)
  for (i = 1; i <= n; i++) {
    c = substr(h, i, 1)
    if (q != "") {
      if (c == q) q = ""
      else {
        if (q == "\"" && c == "\\") { i++; c = substr(h, i, 1) }
        name = name c
      }
    } else if (c == "]") {
      break
    } else if (c == "\"" || c == "\047") {
      q = c
    } else if (c != " " && c != "\t") {
      name = name c
    }
  }
  return name
}
'

# Prints "=" and the value of key in table without its quotes, and "!" when the file sets the
# table in a way a header added under it would break: as an inline table, with dotted keys, or as
# an array of tables. The "=" keeps a value such as "!" apart from the answer "!". A key can be
# bare or quoted, and a value can be a string on one line or a multi-line string over several.
# shellcheck disable=SC2016  # an awk program: $0 is awk's, not the shell's
CONFIG_AWK_GET='
function show(line,   q, i, c, out) {
  found = 1
  sub(/^[^=]*=[ \t]*/, "", line)
  q = substr(line, 1, 3)
  if (q == "\"\"\"" || q == "\047\047\047") {
    multi = q
    out_multi = ""
    take(substr(line, 4), 1)
    return
  }
  q = substr(line, 1, 1)
  if (q == "\"" || q == "\047") {
    out = ""
    for (i = 2; i <= length(line); i++) {
      c = substr(line, i, 1)
      if (c == q) break
      if (q == "\"" && c == "\\") { i++; c = substr(line, i, 1) }
      out = out c
    }
    print "=" out
  } else {
    sub(/[ \t]*(#.*)?\r?$/, "", line)
    print "=" line
  }
  exit
}
# take(part, first): adds one line of a multi-line string to the value, and prints the value once
# the closing quotes are there. TOML drops the line break right after the opening quotes, and in
# a basic string a backslash at the end of a line drops the break and the blanks after it.
function take(part, first,   i, n, c) {
  sub(/\r$/, "", part)
  if (joined) sub(/^[ \t]*/, "", part)
  n = length(part)
  if (joined && n == 0) return
  joined = 0
  for (i = 1; i <= n; i++) {
    if (substr(part, i, 3) == multi) {
      multi = ""
      print "=" out_multi
      exit
    }
    c = substr(part, i, 1)
    if (multi == "\"\"\"" && c == "\\") {
      if (i == n) { joined = 1; return }
      i++
      c = substr(part, i, 1)
    }
    out_multi = out_multi c
  }
  if (!(first && n == 0)) out_multi = out_multi "\n"
}
BEGIN {
  q = "[\"\047]?"
  in_table = "^[ \t]*" q key q "[ \t]*="
  dotted = "^[ \t]*" q table q "[ \t]*\\.[ \t]*" q key q "[ \t]*="
  outside = "^[ \t]*" q table q "[ \t]*[.=]"
}
{
  if (multi != "") { take($0, 0); next }
  classify($0)
  if (header != "") {
    current = header
    if (header == "[" table) blocked = 1
    next
  }
  if (!plain) next
  if (current == table && text ~ in_table) show(text)
  else if (current == "" && text ~ dotted) show(text)
  else if (current == "" && text ~ outside) blocked = 1
}
END {
  if (multi != "") print "=" out_multi
  else if (!found && blocked) print "!"
}
'

# Prints the file with "key = CONFIG_VALUE" right under the table header, or under a new header
# at the end when the file has none.
# shellcheck disable=SC2016  # an awk program: $0 is awk's, not the shell's
CONFIG_AWK_ADD='
{
  print
  last = $0
  classify($0)
  if (!added && header == table) {
    print key " = " ENVIRON["CONFIG_VALUE"]
    added = 1
  }
}
END {
  if (!added) {
    if (NR > 0 && last !~ /^[ \t\r]*$/) print ""
    print "[" table "]"
    print key " = " ENVIRON["CONFIG_VALUE"]
  }
}
'

# config_get <table> <key>: prints "=" and what the config file sets the key to, nothing when it
# sets nothing, and "!" when the file sets the table without a header, where adding a header
# would define the table twice and break the file. Fails when the file cannot be read.
config_get() {
  [ -e "$CONFIG_FILE" ] || return 0
  awk -v table="$1" -v key="$2" "$CONFIG_AWK_CLASSIFY$CONFIG_AWK_GET" "$CONFIG_FILE" 2>/dev/null
}

# config_add <table> <key> <value>: adds the line to the config file and keeps every line that
# is there. The new file is written beside the old one as path.tmp and renamed over it, so a
# failed write never leaves half a file. A config file that is a link is written where the link
# points, so the link stays.
config_add() {
  c_file=$CONFIG_FILE
  c_hops=0
  while [ -L "$c_file" ]; do
    [ "$c_hops" -lt 20 ] || return 1
    c_link=$(readlink "$c_file") || return 1
    case $c_link in
      /*) c_file=$c_link ;;
      *) c_file=$(dirname "$c_file")/$c_link ;;
    esac
    c_hops=$((c_hops + 1))
  done
  mkdir -p "$(dirname "$c_file")" 2>/dev/null || return 1
  # A path.tmp left behind, even a link, is removed rather than written through.
  rm -f "$c_file.tmp" 2>/dev/null || return 1
  c_from=/dev/null
  if [ -e "$c_file" ]; then
    c_from=$c_file
    # Copied first, so the new file keeps the permissions of the one it replaces.
    cp -p "$c_file" "$c_file.tmp" 2>/dev/null || return 1
  fi
  # The braces take the shell's own error when path.tmp cannot be opened, as well as awk's.
  if { CONFIG_VALUE=$3 awk -v table="$1" -v key="$2" "$CONFIG_AWK_CLASSIFY$CONFIG_AWK_ADD" "$c_from" >"$c_file.tmp"; } 2>/dev/null \
    && mv -f "$c_file.tmp" "$c_file" 2>/dev/null; then
    return 0
  fi
  rm -f "$c_file.tmp"
  return 1
}

# toml_string <text>: the text as a TOML basic string, with its backslashes and quotes escaped.
toml_string() {
  printf '"%s"' "$(printf '%s' "$1" | sed 's/[\\"]/\\&/g')"
}

# --- the leader -----------------------------------------------------------------------------

# The leader is written rather than left to the binary's default, because this script installs
# releases from before C-s was the default as well as after it.

leader_prompt() {
  printf '      %s1%s to %s5%s, or enter for %s  ' "$MAUVE" "$OFF" "$MAUVE" "$OFF" "$LEADER_DEFAULT" >&2
}

# choose_leader: asks for the leader and sets CHOSEN. Enter takes the first on the list, which is
# domux's default, and "another key" takes a typed key name.
choose_leader() {
  printf '\n   %s◉%s  Select your leader\n' "$RED" "$OFF" >&2
  printf '      %s1%s  %s\n' "$MAUVE" "$OFF" "$LEADER_DEFAULT" >&2
  printf '      %s2%s  C-a\n' "$MAUVE" "$OFF" >&2
  printf '      %s3%s  C-b\n' "$MAUVE" "$OFF" >&2
  printf '      %s4%s  C-Space\n' "$MAUVE" "$OFF" >&2
  printf '      %s5%s  another key\n' "$MAUVE" "$OFF" >&2
  leader_prompt
  while :; do
    read_key
    # A number picks from the list, and so does pressing the key itself.
    case $KEY in
      ""|1|"$LEADER_DEFAULT") CHOSEN=$LEADER_DEFAULT ;;
      2|C-a) CHOSEN=C-a ;;
      3|C-b) CHOSEN=C-b ;;
      4|C-Space) CHOSEN=C-Space ;;
      5)
        [ -n "$KEY_LINE" ] || printf 'another key\n' >&2
        type_leader
        printf '\n' >&2
        return 0
        ;;
      *)
        # A key that is not on the list is not an answer. A keypress was never echoed, so the
        # prompt is still there; a typed line moved past it, so it is drawn again.
        [ -z "$KEY_LINE" ] || leader_prompt
        continue
        ;;
    esac
    break
  done
  if [ -n "$KEY_LINE" ]; then
    printf '\n' >&2
  else
    printf '%s\n\n' "$CHOSEN" >&2
  fi
}

# type_leader: reads a typed key name into CHOSEN, and asks again until it is one. An empty line
# takes the default. Flow control is off while the line is typed, for the reason
# flow_control_off gives.
type_leader() {
  while :; do
    printf '      key name  ' >&2
    t_name=""
    flow_control_off || true
    KEY_WAITING=yes
    read -r t_name < /dev/tty || printf '\n' >&2
    KEY_WAITING=""
    if [ -z "$t_name" ]; then
      CHOSEN=$LEADER_DEFAULT
      return 0
    fi
    if is_key "$t_name"; then
      CHOSEN=$t_name
      return 0
    fi
    case $t_name in
      *[[:cntrl:]]*) note "write the key's name rather than pressing it, like C-s, M-a or C-Space" ;;
      *) note "$t_name is not a key name; write one like C-s, M-a or C-Space" ;;
    esac
  done
}

LEADER=""
CONFIG_WRITTEN=""
leader_set=$(config_get keys leader) || leader_set=unreadable
case $leader_set in
  unreadable)
    failed_line "could not read $CONFIG_FILE, so no leader was written"
    note "check its permissions, then run this script again"
    ;;
  "!")
    failed_line "no leader written: $CONFIG_FILE sets keys outside a [keys] table"
    note "add leader = $(toml_string "${DOMUX_LEADER:-$LEADER_DEFAULT}") to keys in it by hand"
    ;;
  "")
    leader_how=""
    if [ -n "${DOMUX_LEADER:-}" ]; then
      CHOSEN=$DOMUX_LEADER
    elif asking; then
      choose_leader
    else
      CHOSEN=$LEADER_DEFAULT
      leader_how="the default, since there is no terminal to ask on"
    fi
    if config_add keys leader "$(toml_string "$CHOSEN")"; then
      LEADER=$CHOSEN
      CONFIG_WRITTEN=yes
      done_line "leader $(value "$CHOSEN") written to $CONFIG_FILE"
      [ -z "$leader_how" ] || note "$leader_how"
    else
      failed_line "could not write the leader to $CONFIG_FILE"
      note "add leader = $(toml_string "$CHOSEN") under [keys] in it by hand"
    fi
    ;;
  *)
    leader_set=${leader_set#=}
    LEADER=$leader_set
    done_line "leader $(value "$leader_set") already set in $CONFIG_FILE"
    if [ -n "${DOMUX_LEADER:-}" ] && [ "$DOMUX_LEADER" != "$leader_set" ]; then
      note "DOMUX_LEADER does not replace a leader the config file sets"
    fi
    ;;
esac

# --- stay awake -----------------------------------------------------------------------------

# Stay awake holds the machine awake while agents work, and full mode holds it through a closed
# lid too. On Linux full mode is the line in the config file and nothing else. On macOS it also
# takes a launch daemon and a sudoers line, and writing those is the one thing in domux that
# asks for sudo.

# toggle_note: the key that turns stay awake on and off, which is leader A.
toggle_note() {
  note "press ${LEADER:-the leader} then A inside domux to turn stay awake on or off"
}

# later_note: how to set up full mode after the install.
later_note() {
  if [ "$os" = darwin ]; then
    note "run $domux stay-awake install --full --apply to set it up later"
  else
    note "add mode = \"full\" under [stay_awake] in $CONFIG_FILE to set it up later"
  fi
}

stay_awake_full() {
  if [ "$os" = darwin ]; then
    # sudo asks for a password on the terminal, which is not this script's stdin under
    # "curl | sh", so the command reads the terminal directly when there is one.
    lid_code=0
    if tty_is_open; then
      "$domux" stay-awake install --full --apply </dev/tty >"$tmp/lid.out" 2>&1 || lid_code=$?
    else
      "$domux" stay-awake install --full --apply </dev/null >"$tmp/lid.out" 2>&1 || lid_code=$?
    fi
    if [ "$lid_code" != 0 ]; then
      failed_line "stay awake could not be set up for a closed lid: $(tr '\n' ' ' < "$tmp/lid.out")"
      note "domux still holds the machine awake while the lid is open"
      return 0
    fi
    done_line "the lid setup is installed"
  fi
  if config_add stay_awake mode '"full"'; then
    CONFIG_WRITTEN=yes
    done_line "stay awake set to $(value full) in $CONFIG_FILE"
    toggle_note
  else
    failed_line "could not write stay awake to $CONFIG_FILE"
    note "add mode = \"full\" under [stay_awake] in it by hand"
  fi
}

question="Set up stay awake, so a closed lid does not put this machine to sleep?"
question_note=""
[ "$os" != darwin ] || question_note="it asks for sudo once"

mode_set=$(config_get stay_awake mode) || mode_set=unreadable
case $mode_set in
  unreadable)
    failed_line "could not read $CONFIG_FILE, so stay awake was not set up"
    note "check its permissions, then run this script again"
    ;;
  "!")
    failed_line "stay awake not set up: $CONFIG_FILE sets stay_awake outside a [stay_awake] table"
    note "add mode = \"full\" to stay_awake in it by hand"
    ;;
  "")
    case "${DOMUX_STAY_AWAKE:-}" in
      yes) stay_awake_full ;;
      no)  done_line "stay awake not set up"; later_note ;;
      *)
        if asking && ask "$question" "$question_note"; then
          stay_awake_full
        else
          done_line "stay awake not set up"
          later_note
        fi
        ;;
    esac
    ;;
  *)
    mode_set=${mode_set#=}
    done_line "stay awake already set to $(value "$mode_set") in $CONFIG_FILE"
    toggle_note
    ;;
esac

# A domux that is already running read the config file when it started, so it keeps the old
# leader until it reads the file again.
if [ -n "$CONFIG_WRITTEN" ] && [ -n "$reinstall" ]; then
  note "if domux is already running, run $domux config reload so it reads the new config"
fi

# --- what to do next ------------------------------------------------------------------------

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    printf '\n   %s◉%s  %s is not on your PATH\n' "$RED" "$OFF" "$INSTALL_DIR" >&2
    # shellcheck disable=SC2016  # the line is printed for the reader to paste, so $PATH is literal
    printf '      %sexport PATH="%s:$PATH"%s   in ~/.zshrc or ~/.bashrc\n' "$MAUVE" "$INSTALL_DIR" "$OFF" >&2
    note "fish: fish_add_path $INSTALL_DIR"
    printf '\n' >&2
    ;;
esac

done_line "domux is ready"
printf '\n   %s>%s %srun%s %sdomux%s\n\n' "$MAUVE" "$OFF" "$BRIGHT" "$OFF" "$MAUVE" "$OFF" >&2
