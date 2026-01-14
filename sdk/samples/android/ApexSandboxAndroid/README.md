# ApexSandboxAndroid

Android sandbox app for exercising the Mediation SDK end-to-end. Intended for emulator use only.

## Requirements
- Android Studio Iguana+ (AGP 8.5+)
- JDK 17
- Android Emulator (API 34 recommended)

## Quick start
1. Open this folder in Android Studio (File → Open → select `ApexSandboxAndroid`).
2. Let Gradle sync. If prompted, allow Android Studio to create/upgrade the Gradle wrapper.
3. Create an Android emulator (Pixel 6a, API 34).
4. Run configuration: "app" → Run on the emulator.

## Emulator-only guard
- The app checks `Build.*` heuristics at runtime via `EmulatorGuard.isRunningOnEmulator()` and terminates on physical devices.

## Staging configuration
- Configure staging endpoints and placement IDs in `app/src/main/assets/sandbox_config.json`.
- Consent defaults (GDPR/CCPA/COPPA/LAT/Test Mode) persist in `SharedPreferences` (`sandbox_prefs`).

## Screen & Controls → Checklist mapping (0.0.2)
- SDK Controls
  - Initialize → "single initialize call" validation (idempotent status updates).
- Network Behavior
  - Selector: A (always fill), B (random no-fill), C (slow timeout) → Fake network behaviors for happy-path and error cases (0.0.2 bullets 54–55).
  - Airplane Mode toggle → Simulate network unreachable (0.0.2 bullet 55).
  - Invalid Placement toggle → Force invalid placement errors (0.0.2 bullet 55).
- Placements
  - Interstitial A/B: Load/Show buttons → Validate load→show→callback order (0.0.2 bullet 54).
  - Rewarded A: Load/Show buttons → Validate callback sequence and single-show guard (0.0.2 bullets 54, 56).
- Banner
  - Start/Stop: placeholder banner refresh every 5s → Layout stability (0.0.2 bullet 54).
- Privacy & Test Toggles
  - GDPR, CCPA, COPPA, LAT, Test Mode → Persisted to `SharedPreferences` and reflected in outbound metadata/logs (0.0.2 bullet 57).
- Request Log
  - Rolling log of last ~200 events with Clear button → Observe flows and errors; use for soak run (0.0.2 bullets 54–58).

## Soak run (30 minutes)
Goal: exercise placements repeatedly to surface leaks/ANRs.

Options:
- Manual: repeatedly tap Load/Show for Interstitial A/B and Rewarded A at a steady cadence. Rotate screen and background/foreground the app.
- Assisted (ADB): use the helper script below as a starting point to automate basic interactions.

### scripts/soak.sh (starter)
- Runs for ~30 minutes by default, sending back/foreground and a few UI taps. Adjust coordinates for your emulator skin if needed.

```bash
./scripts/soak.sh  # from this project directory
```

Notes:
- Monitor with Android Studio Profiler (CPU/memory). Expect no crashes, ANRs, or runaway memory (0.0.2 bullet 58).
- Logs should show consistent callback ordering; only one Show active at a time (0.0.2 bullet 56).
