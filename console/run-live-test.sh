#!/bin/bash
LIVE_E2E=1 \
CONSOLE_BASE_URL=https://console.apexmediation.ee \
LIVE_CONSOLE_USERNAME="advertiser-live@test.com" \
LIVE_CONSOLE_PASSWORD="Test1234!" \
npx playwright test --project=chromium tests/e2e/live-login.spec.ts --reporter=list --timeout=180000
