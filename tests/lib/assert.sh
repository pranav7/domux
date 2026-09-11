#!/bin/sh
# Assertions for the shell test runners under tests/. Source this file; do not run it.
#
# A runner defines functions named test_<behavior>_<condition> and ends with
#   run_tests test_a test_b ...
# Each assertion records a failure and continues, so one runner reports every broken case.
# run_tests prints "ok <name>" or "FAIL <name>: <reason>" lines on stderr and exits 1 when any
# test failed. Runners use "set -u" and never "set -e", so a failing command under test does
# not abort the runner; capture exit codes with: cmd; code=$?

FAILED=0
PASSED=0
CURRENT=""
CASE_FAILED=0

fail() {                  # fail <reason>
  printf 'FAIL %s: %s\n' "$CURRENT" "$*" >&2
  FAILED=$((FAILED + 1))
  CASE_FAILED=1
}

assert_eq() {             # assert_eq <want> <got> <label>
  [ "$1" = "$2" ] || fail "$3: want '$1', got '$2'"
}

assert_contains() {       # assert_contains <haystack> <needle> <label>
  case "$1" in
    *"$2"*) ;;
    *) fail "$3: '$2' not found in: $1" ;;
  esac
}

assert_not_contains() {   # assert_not_contains <haystack> <needle> <label>
  case "$1" in
    *"$2"*) fail "$3: '$2' found in: $1" ;;
  esac
}

assert_file() {           # assert_file <path> <label>
  [ -f "$1" ] || fail "$2: $1 is not a file"
}

assert_no_file() {        # assert_no_file <path> <label>
  [ ! -e "$1" ] || fail "$2: $1 exists"
}

assert_exit() {           # assert_exit <want-code> <got-code> <label>
  [ "$1" = "$2" ] || fail "$3: want exit $1, got $2"
}

# The loop variable carries the ASSERT_ prefix every runner-local name avoids, because a test
# helper that sets a plain "t" would otherwise take over this loop: POSIX sh has no local
# variables, so every name here is shared with the tests it runs.
run_tests() {             # run_tests <test function names>
  for ASSERT_CASE in "$@"; do
    CURRENT=$ASSERT_CASE
    CASE_FAILED=0
    "$ASSERT_CASE"
    if [ "$CASE_FAILED" = 0 ]; then
      PASSED=$((PASSED + 1))
      printf 'ok %s\n' "$CURRENT" >&2
    fi
  done
  printf '%s passed, %s failed\n' "$PASSED" "$FAILED" >&2
  [ "$FAILED" = 0 ]
}
