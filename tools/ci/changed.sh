#!/bin/sh
# Fast, diff-scoped CI: see tools/ci/changed.py.
set -eu
exec python3 "$(dirname -- "$0")/changed.py" "$@"
