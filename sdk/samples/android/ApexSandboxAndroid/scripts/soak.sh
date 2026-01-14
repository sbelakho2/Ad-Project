#!/usr/bin/env bash
set -euo pipefail

# 30-minute soak runner using instrumentation argument SOAK_MINUTES.
# Usage:
#   ./scripts/soak.sh               # default 30 minutes
#   ./scripts/soak.sh 10            # custom minutes
#
# Prereq: emulator running and visible to `adb devices`.

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
APP_ID=com.apex.sandbox.android
TEST_RUNNER=androidx.test.runner.AndroidJUnitRunner
SOAK_MINUTES=${1:-30}

echo "[soak] Building debug app and androidTest..."
./gradlew -p "$ROOT_DIR" :app:assembleDebug :app:assembleDebugAndroidTest --no-daemon

APK_TEST_PATH=$(find "$ROOT_DIR/app/build/outputs/apk/androidTest/debug" -name "*.apk" | head -n1 || true)
if [[ -z "$APK_TEST_PATH" ]]; then
  echo "[soak] ERROR: androidTest APK not found."
  exit 1
fi

echo "[soak] Ensuring emulator is connected..."
adb start-server >/dev/null
adb devices

echo "[soak] Clearing app data..."
adb shell pm clear ${APP_ID}.debug || true

echo "[soak] Starting $SOAK_MINUTES minute soak via instrumentation..."
adb shell am instrument -w \
  -e SOAK_MINUTES "$SOAK_MINUTES" \
  -e class com.apex.sandbox.SoakTest \
  ${APP_ID}.debug.test/${TEST_RUNNER}

STATUS=$?
if [[ $STATUS -ne 0 ]]; then
  echo "[soak] Instrumentation finished with status $STATUS" >&2
  exit $STATUS
fi

echo "[soak] Done. Review Android Studio profiler for CPU/memory and logcat for errors."
