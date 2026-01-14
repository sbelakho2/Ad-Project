#!/bin/bash
set -euo pipefail

: "${LIVE_CONSOLE_USERNAME:?Set LIVE_CONSOLE_USERNAME in env}"
: "${LIVE_CONSOLE_PASSWORD:?Set LIVE_CONSOLE_PASSWORD in env}"

LIVE_E2E=1 \
CONSOLE_BASE_URL=${CONSOLE_BASE_URL:-https://console.apexmediation.ee} \
LIVE_CONSOLE_USERNAME="${LIVE_CONSOLE_USERNAME}" \
LIVE_CONSOLE_PASSWORD="${LIVE_CONSOLE_PASSWORD}" \
npx playwright test --project=chromium tests/e2e/live-login.spec.ts --reporter=list --timeout=180000
