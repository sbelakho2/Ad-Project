# Contributing — iOS SDK API Baseline and Breaking Changes

This document explains how we guard the public API surface of the iOS SDK using Swift’s API digester, and how to intentionally update the baseline when making breaking changes.

## Overview

We use `swift-api-digester` to detect breaking changes to the public API of the module `RivalApexMediationSDK`.

- The baseline JSON is stored at:
  - `sdk/core/ios/.api-baseline/RivalApexMediationSDK.public.json`
- CI compares the current build against the baseline on pull requests (see `.github/workflows/ci-all.yml`).
- If breaking changes are detected, the PR fails with a diff attached as an artifact (`ios-api-diff`).

## Prerequisites

- macOS (runner or local machine)
- Xcode 15.x toolchain (the CI runs on `macos-13`)
- Swift toolchain available via `xcrun`

## Generating or refreshing the baseline

Use the helper script:

```
sdk/core/ios/scripts/generate-api-baseline.sh
```

This will:
1) Build the module in release mode to ensure it compiles.
2) Generate `RivalApexMediationSDK.public.json` under `.api-baseline/`.

Commit the resulting JSON file to the same branch as your changes.

## Breaking changes policy

Follow Semantic Versioning:

- Patch (X.Y.z): No public API changes; bug fixes only.
- Minor (X.y.Z): Backward-compatible additions to the API.
- Major (x.Y.Z): Backward-incompatible API changes.

If you intend to introduce a breaking change:

1. Update the planned version (major bump) in release notes/changelog.
2. Regenerate the baseline JSON using the script above.
3. Commit the updated baseline with your PR.
4. Clearly document the migration steps in the PR description and CHANGELOG.

## CI notes

- The CI step will be enforced only when the baseline file exists. If it is missing, CI logs will instruct you to generate and commit it.
- The CI uses `iphoneos` SDK and `arm64-apple-ios14.0` as the target triple to match our minimum supported iOS version.

## Troubleshooting

- If `swift-api-digester` is not found, ensure Xcode command line tools are selected:
  - `xcode-select -p`
  - `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`
- If you see false positives due to SDK differences, confirm your local Xcode version matches the CI runner (macOS 13 with Xcode 15.x).
