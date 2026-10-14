#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_DIR="$ROOT_DIR"
MAX_SIZE_KB=100

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# Unity packages are distributed as compressed archives, so we mimic that footprint.
ARCHIVE_PATH="$TMP_DIR/runtime.tgz"
tar -czf "$ARCHIVE_PATH" -C "$PACKAGE_DIR" Runtime >/dev/null
actual_size=$(du -sk "$ARCHIVE_PATH" | awk '{print $1}')
if [[ "$actual_size" -gt "$MAX_SIZE_KB" ]]; then
  echo "Compressed runtime exceeds ${MAX_SIZE_KB}KB budget (actual ${actual_size}KB)"
  exit 1
fi

echo "Runtime compressed size OK (${actual_size}KB <= ${MAX_SIZE_KB}KB)"

pushd "$ROOT_DIR/Tests" >/dev/null
 dotnet test
popd >/dev/null

echo "Unity SDK constraints satisfied."
