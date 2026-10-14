#!/usr/bin/env bash
set -euo pipefail

# This script generates the Swift API digester baseline for the iOS SDK.
# It should be run on macOS with Xcode toolchain installed.
# Output: .api-baseline/RivalApexMediationSDK.public.json

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IOS_ROOT="${SCRIPT_DIR}/.."
cd "$IOS_ROOT"

TARGET_MODULE="RivalApexMediationSDK"
BASELINE_DIR=".api-baseline"
MIN_TARGET="arm64-apple-ios14.0"

mkdir -p "$BASELINE_DIR"

echo "Building $TARGET_MODULE (release) to ensure module is available..."
swift build -c release

echo "Resolving iPhoneOS SDK path..."
SDK_PATH=$(xcrun --show-sdk-path --sdk iphoneos)

echo "Generating Swift API digester baseline for $TARGET_MODULE -> $BASELINE_DIR/${TARGET_MODULE}.public.json"
xcrun swift-api-digester \
  -sdk "$SDK_PATH" \
  -dump-sdk \
  -module "$TARGET_MODULE" \
  -target "$MIN_TARGET" \
  -o "$BASELINE_DIR/${TARGET_MODULE}.public.json"

echo "Baseline generated at $BASELINE_DIR/${TARGET_MODULE}.public.json"
