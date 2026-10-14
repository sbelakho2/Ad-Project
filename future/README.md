# Future surface

This tree holds the mediation SDKs (Android, iOS, tvOS, Unity, web,
react-native, flutter and the adapter dev kit) that are not part of the
current product. The current product is the no-SDK autopilot that sits on top
of AppLovin MAX, Unity LevelPlay and AdMob, so the SDKs are parked here,
kept intact, and reserved for the future.

- Nothing in the current product builds or imports from this tree.
- The coverage classifier marks `future/**` as `FUTURE_SURFACE`, so the
  gate stack proves the files exist and are tracked without treating them as
  product code.
- The platform suites in `future/sdk/web` and the other packages still run
  with their own package managers when the future needs them; CI does not
  build this tree today.

Do not delete anything under `future/` without a decision to retire the SDKs
for good.
