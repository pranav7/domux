#!/bin/sh
# Builds the hand-written fixtures. printf interprets \033 as ESC.
set -eu
cd "$(dirname "$0")"

# Attributes, 16 colors, 256 colors, truecolor.
{
  printf '\033[1mbold\033[0m \033[2mdim\033[0m \033[3mitalic\033[0m \033[4munderline\033[0m \033[7minverse\033[0m \033[9mstrike\033[0m\r\n'
  printf '\033[4:3mcurly\033[0m \033[21mdouble\033[0m \033[53moverline\033[0m \033[5mblink\033[0m \033[8mhidden\033[0m\r\n'
  i=0; while [ $i -lt 16 ]; do printf '\033[38;5;%dm%02d\033[0m ' $i $i; i=$((i+1)); done; printf '\r\n'
  i=16; while [ $i -lt 32 ]; do printf '\033[48;5;%dm  \033[0m' $i; i=$((i+1)); done; printf '\r\n'
  printf '\033[38;2;255;128;0mtruecolor fg\033[0m \033[48;2;30;30;46mtruecolor bg\033[0m \033[58;2;255;0;0m\033[4mcolored underline\033[0m\r\n'
  printf '\033[31;1;4mred bold underline\033[39mstill bold underline\033[0m\r\n'
} > sgr-attributes.80x24.bytes

# Cursor movement, erase, insert and delete.
{
  printf 'line one\r\nline two\r\nline three\r\n'
  printf '\033[1;1Hxxx'                 # overwrite start of line 1
  printf '\033[2;6H\033[K'              # erase to end of line 2
  printf '\033[3;1H\033[2@'             # insert two blank cells at start of line 3
  printf '\033[10;40Hcenter\033[s\033[20;1Hbottom\033[u!'   # save and restore
  printf '\033[12;1H\033[1J'            # erase from top to cursor, then nothing else
  printf '\033[15;1Ha\tb\tc\r\n'        # tab stops
  printf '\033[16;1H12345\033[3D\033[P' # delete one char under cursor
} > cursor-and-erase.80x24.bytes

# Scroll region with scrolling in both directions.
{
  i=1; while [ $i -le 24 ]; do printf 'row %02d\r\n' $i; i=$((i+1)); done
  printf '\033[5;10r\033[10;1H\n\n'     # region rows 5-10, two scrolls up from the bottom
  printf '\033[5;1H\033M'               # one reverse index at the top of the region
  printf '\033[r\033[24;1H'             # reset region, park cursor
} > scroll-region.80x24.bytes

# Alternate screen: main content must be intact after leaving.
{
  printf 'main screen line\r\nsecond main line\r\n'
  printf '\033[?1049h\033[2J\033[HALT SCREEN CONTENT\033[?1049l'
  printf 'after alt'
} > alt-screen.80x24.bytes

# Wide characters, emoji, combining marks, wrap at a wide boundary.
{
  printf '漢字テスト 中文 한국어\r\n'
  printf '👩‍💻 👨‍👩‍👧 🇯🇵 ❤️ 👍🏽 ☃ ☃️\r\n'
  printf 'e\314\201 (e + combining acute) n\314\203 (n + tilde)\r\n'
  printf '─│┌┐└┘├┤┬┴┼ ▀▄█▌▐ ░▒▓\r\n'
  i=0; while [ $i -lt 79 ]; do printf 'a'; i=$((i+1)); done; printf '漢tail\r\n'   # wide char at column 79 wraps whole
  printf '\033[?7l'; i=0; while [ $i -lt 90 ]; do printf 'w'; i=$((i+1)); done; printf '\033[?7h\r\n'  # autowrap off
} > wide-and-emoji.80x24.bytes

# Wrap and pending wrap state.
{
  i=0; while [ $i -lt 100 ]; do printf '%d' $((i % 10)); i=$((i+1)); done
  printf '\r\n'
  i=0; while [ $i -lt 80 ]; do printf 'x'; i=$((i+1)); done; printf '\b\bYZ\r\n'  # backspace at pending wrap
  i=0; while [ $i -lt 80 ]; do printf 'y'; i=$((i+1)); done; printf '\033[1mZ'      # attribute across wrap
} > wrap-and-pending.80x24.bytes

ls -1 *.bytes
