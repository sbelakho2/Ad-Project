#!/usr/bin/env bash
set -euo pipefail

# Fetch OMSDK binaries for test-only usage (BYO production stays unbundled).
# Override URLs with OMSDK_IOS_URL / OMSDK_ANDROID_URL if newer versions are needed.
# Defaults use publicly hosted artifacts that have been reachable in CI:
# - iOS: Appodeal-hosted OMSDK xcframework zip (1.5.2)
# - Android: Maven Central fyber OMSDK AAR (1.5.1)
IOS_URL="${OMSDK_IOS_URL:-https://s3-us-west-1.amazonaws.com/appodeal-ios/external-sdks/OMSDK_Appodeal/1.5.2/OMSDK_Appodeal.zip}"
ANDROID_URL="${OMSDK_ANDROID_URL:-https://repo1.maven.org/maven2/com/fyber/omsdk/om-sdk/1.5.1/om-sdk-1.5.1.aar}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IOS_DIR="$ROOT/core/ios/Vendor/OMSDK"
ANDROID_DIR="$ROOT/core/android/Vendor/OMSDK"
UNITY_DIR="$ROOT/core/unity/Vendor/OMSDK"

mkdir -p "$IOS_DIR" "$ANDROID_DIR" "$UNITY_DIR"

fetch() {
  local url="$1"; local dest="$2"; local label="$3"
  echo "[fetch-omsdk] downloading $label from $url"
  curl -fL "$url" -o "$dest"
}

fetch "$IOS_URL" "$IOS_DIR/omsdk-ios.zip" "iOS OMSDK"
fetch "$ANDROID_URL" "$ANDROID_DIR/omsdk-android.aar" "Android OMSDK"

# Unity consumes the Android helper; reuse the downloaded artifact.
cp "$ANDROID_DIR/omsdk-android.aar" "$UNITY_DIR/omsdk-android.aar"

echo "[fetch-omsdk] downloaded test-only OMSDK artifacts to:"
printf '  - %s\n' "$IOS_DIR/omsdk-ios.zip" "$ANDROID_DIR/omsdk-android.zip" "$UNITY_DIR/omsdk-android.zip"
