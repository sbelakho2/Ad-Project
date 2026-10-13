# Moloco Replacement Plan

_Last updated: 2025-11-19_

## Goal
Fully replace the deprecated InMobi integration with Moloco across every surface (backend bidders, SDK registries, documentation, and observability), keeping the overall network count at 15 while restoring parity across platforms.

## Scope
1. **Backend auction service** – ship a production-ready Moloco bidder with configuration + conformance tests.
2. **SDKs (Android/iOS/CTV/Unity/Web)** – expose Moloco in every adapter registry with stubs or real adapters.
3. **Tooling & docs** – refresh API key guidance, transparency docs, and readiness checklists to mention Moloco only.
4. **Verification** – add regression tests and manual checks covering the new bidder plus registry discovery across platforms.

## Workstream checklists

### 1. Backend auction (Go)
- [x] Implement bidder at `backend/auction/internal/bidders/moloco.go` with retry + breaker.
- [x] Add conformance coverage in `backend/auction/internal/bidders/moloco_conformance_test.go` (no-fill, retry, timeout, circuit).
- [ ] Wire `NewMolocoAdapter` into the runtime requester factory / dependency injection so `AuctionEngine` can instantiate it when `adapterName=="moloco"`.
- [ ] Document ENV / secrets required for Moloco (seat ID, API key) in `backend/Adapters.md` + sample config (`backend/src/services/openrtbEngine.ts`).
- [ ] Update infra manifests (Helm/self-hosted) with placeholders for `MOLOCO_SEAT_ID`, `MOLOCO_API_KEY`.

### 2. Android SDK
- [x] Add placeholder adapter at `sdk/core/android/src/main/kotlin/com/rivalapexmediation/adapter/moloco/Adapter.kt` so discovery remains at 15.
- [x] Register package in `sdk/core/android/src/main/kotlin/adapter/AdapterRegistry.kt`.
- [ ] Replace stub with vendor SDK bridge using `com.moloco.sdk:moloco-ads:1.2.0` per `AdapterRebuildPlan.md`.
- [ ] Add Robolectric/instrumented tests using Moloco demo placements and ensure paid events map into `AnalyticsEvent.AdRevenue`.

### 3. iOS / tvOS SDK
- [x] Ship `MolocoAdapter` stub class in `sdk/core/ios/Sources/Adapter/StubAdapters.swift`.
- [x] Register Moloco in `AdapterRegistry.registerBuiltInAdapters()`.
- [ ] Implement Swift adapter backed by the vendor SDK and wire delegate callbacks -> `AdapterDelegate`.
- [ ] Expand unit tests to assert initialization report includes Moloco once configured.

### 4. Unity + CTV + Web
- [x] Unity: update `Packages/com.rivalapexmediation.sdk/Runtime/Adapters/AdapterRegistry.cs` to enumerate `moloco` instead of `inmobi`.
- [x] CTV Android TV registry already lists `moloco`; no further action required until native adapter exists.
- [x] Web SDK: `packages/web-sdk/src/adapters.ts` already exposes `moloco` in `SUPPORTED_NETWORKS`.
- [ ] Console/Website settings should display Moloco-specific credential inputs (seat ID, API key) once backend config model lands.

### 5. Docs & API keys
- [x] Refresh network inventory docs (`docs/Adapters/SUPPORTED_NETWORKS.md`, `docs/Web/INTEGRATION_README.md`, sandbox readiness) to replace InMobi with Moloco.
- [ ] Update `docs/Internal/Deployment/API_KEYS_AND_INTEGRATIONS_GUIDE.md` with Moloco credential workflow (portal URL, sandbox steps, rate limits).
- [ ] Add Moloco-specific troubleshooting to `docs/Internal/Runbooks/Adapters/Moloco.md` (create file) covering bid statuses, rate limiting, and creative review delays.

## Verification plan
1. **Backend** – `cd backend/auction && go test ./internal/bidders -run Moloco` after wiring DI. Add synthetic load-test scenario hitting `/v1/auction` with `adapters:["moloco"]` while stubbing the Moloco endpoint via `httptest.Server`.
2. **SDKs** – run `./gradlew :sdk:core:android:testDebugUnitTest` and `swift test` once Moloco adapters move beyond stubs.
3. **Unity** – smoke test the package in the sample scene; ensure `AdapterRegistry.GetSupportedNetworks()` returns 15 names with `moloco` present.
4. **Docs** – lint (the `lint:docs` script is not yet wired; track it in the docs-reference contract backlog) and spot-check the rendered Markdown for typos.

## Risks & mitigations
- **Credential onboarding delay:** block-release until Moloco issues sandbox seat + API keys. Mitigation: keep `test_endpoint` override path in bidder for local tests.
- **Bid payload drift:** the REST contract may change; keep `molocoResp` parsing guarded and log unknown fields into observability pipeline.
- **Parasitic retries:** ensure we honor any `Retry-After` headers once production docs confirm support; placeholder request currently retries only on transient failures.

## Decision log
- 2025-11-19: Chose Moloco as the replacement network because of faster S2S onboarding and responsive support compared to InMobi. All teams aligned on removing InMobi SDKs entirely.
