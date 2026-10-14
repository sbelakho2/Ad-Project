#!/usr/bin/env bash
set -euo pipefail
PORT=${PORT:-8124}
FLAG=${FLAG:-false}
BASE="http://127.0.0.1:${PORT}/api/v1"
LOG_DIR=$(mktemp -d)
LOG_FILE="$LOG_DIR/stub.log"
cleanup() {
  if [[ -n "${STUB_PID:-}" ]]; then
    kill "$STUB_PID" >/dev/null 2>&1 || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT
METRICS_ENABLED="$FLAG" METRICS_STUB_PORT="$PORT" python3 "$(dirname "$0")/metrics_stub.py" >"$LOG_FILE" 2>&1 &
STUB_PID=$!
sleep 1
API_BASE_URL="$BASE" swift run --package-path sdk/ctv/tvos CTVSDKDevProbe
sleep 1
cat "$LOG_FILE"
