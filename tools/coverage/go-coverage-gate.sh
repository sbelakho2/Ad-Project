#!/bin/sh
# go-coverage-gate.sh — fails unless every function reports >= FLOOR% (including the total line).
# Usage: go-coverage-gate.sh <cover.func> <floor>
set -eu
FILE="${1:?usage: go-coverage-gate.sh <cover.func> <floor>}"
FLOOR="${2:?usage: go-coverage-gate.sh <cover.func> <floor>}"
[ -f "$FILE" ] || { echo "go coverage file not found: $FILE"; exit 1; }

OFFENDERS="$(awk -v floor="$FLOOR" '
  match($0, /[0-9]+\.[0-9]%$/) {
    pct = substr($0, RSTART, RLENGTH - 1) + 0
    if (pct < floor + 0) print "  - " $0
  }' "$FILE")"

if [ -n "$OFFENDERS" ]; then
  echo "GO COVERAGE GATE FAILED (floor=${FLOOR}%) — offenders:"
  printf '%s\n' "$OFFENDERS"
  exit 1
fi

ROWS="$(awk 'match($0, /[0-9]+\.[0-9]%$/) { n += 1 } END { print n + 0 }' "$FILE")"
echo "go coverage gate PASS (floor=${FLOOR}%): ${ROWS} rows checked incl. total"
