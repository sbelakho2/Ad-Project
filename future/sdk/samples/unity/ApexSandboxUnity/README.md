# ApexSandboxUnity

Unity sandbox project (single scene) to exercise the Unity SDK wrapper and native bridges for Android and iOS. Designed for emulator/simulator runs.

Requirements
- Unity 2022.3 LTS (or later)
- Unity Hub (recommended)

Project layout
- Assets/
  - Scenes/
    - Main.unity (single scene)
  - Scripts/
    - SandboxController.cs (Init/Load/Show UI + status console)
    - SdkBridge.cs (Unity→native bridge with simulated fallbacks)
    - Enums.cs, ConsentPayload.cs, Logger.cs (helpers)
  - Resources/
    - SandboxConfig.json (placements, initial consent, appId, testMode)
  - Editor/
    - BuildScripts.cs (menu items to export Android/iOS builds)
- Packages/manifest.json (minimal dependencies)

What’s implemented
- Init (idempotent), Load/Show Interstitial, Load/Show Rewarded
- Optional Banner visibility toggle (simulated)
- Consent toggles (GDPR, CCPA, COPPA, Test Mode) propagated to the bridge
- Status console with rolling logs and last error
- Error mapping to enums: no-fill, timeout, network, invalid-placement
- Exactly-once show callbacks with presentation guard

Getting started
1. Open Unity Hub → Open → select this `ApexSandboxUnity` folder.
2. Open `Assets/Scenes/Main.unity` and press Play to try the flows in Editor (simulated bridge).

Export builds (Editor menus)
- Android (Gradle): ApexSandbox → Export Android (Gradle)
- iOS (Xcode): ApexSandbox → Export iOS (Xcode)
  - Outputs under `Builds/Android/` and `Builds/iOS/`

Notes
- Bridge forwards consent and init to native layers when present; otherwise logs (Editor/simulated mode).
- Placement IDs in `Assets/Resources/SandboxConfig.json` mirror the Android/iOS sandbox apps.
- Use emulators/simulators; physical devices are not required for this sandbox.

Related checklist
- See `docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md` section 0.0.4 Unity Test Project.
