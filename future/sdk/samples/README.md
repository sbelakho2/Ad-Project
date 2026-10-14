# Test Apps

This folder contains platform-specific sandbox apps used to validate the SDK end‑to‑end across emulators/simulators only.

Platforms:
- Android — `ApexSandboxAndroid`
- Android TV — `ApexSandboxAndroidTV`
- iOS (Simulator only) — `ApexSandboxiOS`
- tvOS (Simulator only) — `ApexSandboxTVOS`
- Web — `apex-sandbox-web`
- Unity — `ApexSandboxUnity`

Emulator/Simulator‑Only Policy:
- All apps include runtime or compile-time guards preventing execution on physical devices or non-local web hosts.
- Use staging endpoints and test placement IDs only.

Refer to docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md sections 0.0.2–0.0.4 and related CTV/Web items for the full test matrix these apps support.
