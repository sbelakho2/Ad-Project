# ApexSandboxiOS

iOS sandbox app (Simulator only) for exercising the Rival Apex Mediation SDK flows end‑to‑end.

What it includes
- Initialize (idempotent)
- Load/Show Interstitial, Load/Show Rewarded
- Consent toggles: GDPR, CCPA, COPPA, Test mode
- ATT request button (iOS 14+)
- Debug overlay with config version, last error, and rolling logs

Requirements
- Xcode 16+
- XcodeGen (`brew install xcodegen`)

SDK integration
- Uses a local Swift Package dependency on `sdk/core/ios` (product `RivalApexMediationSDK`). No CocoaPods needed.

Quick start
1. `cd "Test Apps/ios/ApexSandboxiOS"`
2. `xcodegen generate`
3. Open `ApexSandboxiOS.xcodeproj` in Xcode
4. Select an iOS Simulator (e.g., iPhone 15) and Run

Configuration
- Edit `Sources/SandboxConfig.json` to adjust placements and a staging base URL
- ATT string is defined in `Sources/Info.plist` (`NSUserTrackingUsageDescription`)

Notes
- The app terminates immediately if built for a physical device; it is Simulator‑only by design (see `#if targetEnvironment(simulator)`).
- Presentation uses debug placeholders provided by the SDK facades for ease of testing.
