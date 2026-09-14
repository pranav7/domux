#!/bin/sh
# Builds this checkout and hands the running server over to the build, keeping every pane
# (docs/decisions/0046-an-upgrade-keeps-every-pane.md).
#
# Usage: scripts/dev/deploy.sh [--no-pull]
#
#   1. Fast-forwards the checked out branch from its upstream. --no-pull builds the checkout
#      as it is, which is how a contributor tries a change that is not pushed.
#   2. Builds the release binary.
#   3. Points domux on the PATH at the build: ~/.local/bin/domux, where the installer puts it,
#      and ~/bin/domux too when something is already there, since a hook can run that path.
#   4. Runs the build's server upgrade, which starts a server when none is running.
set -eu

pull=yes
for arg in "$@"; do
  case $arg in
    --no-pull) pull=no ;;
    -h | --help)
      sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      printf 'deploy: unknown argument %s; run it with --help\n' "$arg" >&2
      exit 2
      ;;
  esac
done

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

if [ "$pull" = yes ]; then
  git pull --ff-only
fi
cargo build --release --locked -p domux
build="$root/target/release/domux"

# Makes $1 a link to the build, saying so when it changes anything.
link() {
  if [ -L "$1" ] && [ "$(readlink "$1")" = "$build" ]; then
    return 0
  fi
  if [ -e "$1" ] && [ ! -L "$1" ]; then
    printf 'deploy: %s was a binary of its own; it is now a link to the build\n' "$1" >&2
  fi
  mkdir -p "$(dirname "$1")"
  rm -f "$1"
  ln -s "$build" "$1"
  printf 'deploy: %s now runs %s\n' "$1" "$build" >&2
}

link "$HOME/.local/bin/domux"
if [ -e "$HOME/bin/domux" ] || [ -L "$HOME/bin/domux" ]; then
  link "$HOME/bin/domux"
fi

exec "$build" server upgrade
