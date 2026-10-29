Android BYO Mediation SDK Fixes & Enhancements:

---

## 2025-11-23 — Android SdkMode gating + adapter metadata
- `sdk/core/android/src/main/kotlin/MediationSDK.kt` now enforces the BYO checklist rule that S2S auctions only run when every enabled network both declares `supportsS2S` and provides the required credentials. BYO mode stays the default; HYBRID/MANAGED still respect `enableS2SWhenCapable` plus a configured auction API key.
- Remote config models (`sdk/core/android/src/main/kotlin/Models.kt`) gained `AdapterConfig.supportsS2S` and `AdapterConfig.requiredCredentialKeys`, allowing backend config payloads to describe S2S capability and the secrets the SDK must receive before enabling auctions.
- Documentation cross-links updated (`docs/Internal/Development/BYO_SDK_PRODUCTION_CHECKLIST.md`, `CHANGELOG.md`) so future releases can track this requirement’s implementation details.

## 2025-11-23 — iOS BYO lifecycle + consent plumbing
- `sdk/core/ios/Sources/MediationSDK.swift` now exposes `setConsent`, `currentConsent()`, and snapshot-backed consent summaries while injecting `apx_consent_state` + personalization flags into every adapter init/load call so BYO partners always receive normalized GDPR/CCPA/COPPA/ATT state.
- `sdk/core/ios/Sources/Facades/BelAds.swift`, `BelInterstitial.swift`, `BelRewarded*.swift`, and `BelAppOpen.swift` all route through the centralized cache and `BelAdEventListener`, mirroring Android’s lifecycle (load/show/reward/fail) on the main actor.
- `docs/Customer-Facing/SDKs/IOS_QUICKSTART.md` now documents the shared listener pattern and the new consent plumbing; `SDK_FIXES.md` captures the implementation notes for BYO readiness.
- Tests (`ConsentManagerTests`, new `ConsentPlumbingIntegrationTests`, and facade specs) lock in the adapter payloads, consent redaction, and listener error flows.

> **Canonical requirements now live in** `docs/Internal/Development/BYO_SDK_PRODUCTION_CHECKLIST.md`. The content below stays focused on changelog-style implementation notes; consult the checklist for the authoritative to-do list, acceptance criteria, and platform-specific expectations.

### BYO Certification Checklist (pointer)

- Runtime + adapter requirements, consent plumbing, OMSDK, lifecycle guarantees, tooling, and Managed Demand seam design are documented in [`docs/Internal/Development/BYO_SDK_PRODUCTION_CHECKLIST.md`](docs/Internal/Development/BYO_SDK_PRODUCTION_CHECKLIST.md).
- Unity parity requirements (package layout, public API, platform bridges, samples, footprint gates) are consolidated in the same checklist under “Unity SDK Requirements.”
- Any updates to scope/acceptance must be captured in that doc first; copy only high-level summaries here if needed for release notes.

#### Unity P2 implementation snapshot (Nov 2025)
- **Config-as-Code** — Implemented via `Runtime/Core/ConfigCodec.cs` with System.Text.Json + Unity `JsonUtility` support, Editor tooling in `Editor/ConfigAsCodeWindow.cs`, and coverage in `Tests/ConfigCodecTests.cs`. Exported configs are HMAC signed using `ConfigSignature`, and imports are signature-verified before constructing `ApexConfig`.
- **Single-entry bootstrap** — `Runtime/ApexMediationEntryPoint.cs` lets developers drop a single MonoBehaviour into a scene, point it at a signed config TextAsset, optionally provide adapter credentials, and (in Editor) automatically spawn the Mediation Debugger overlay for sanitized traces/proofs.
- **Cryptographic transparency** — `TransparencyLedger`, `TransparencyProof`, and the updates in `MediationSDK` maintain a rolling hash chain for selection events. `MediationDebuggerOverlay` surfaces the latest proof hash in-editor so developers can trace decisions against the ledger.
- **Automated size/test gates** — `sdk/core/unity/scripts/check_package_constraints.sh` now packages `Runtime/` as a compressed tar (mirrors Unity package distribution), enforces the 100KB compressed budget, and runs the .NET test suite so CI can guard both footprint and functionality with a single job.

What to avoid now (until Managed Demand)

No payment rails/ledger in Unity package.

No exchange/DSP logic in Unity runtime.

No auto-collect of revenue from networks (unless customer adds their own connectors in your console—still recommended to keep Unity package free of that).

Acceptance checklist (Unity BYO package)

Init & consent: Works on device; consent matrix verified; no secret leakage.

Load/Show: Interstitial, Rewarded, Banner with single-use/TTL guarantees; callbacks main-thread and deterministic; rotation safe.

Adapters: Vendor SDK render paths (no WebView placeholder).

OMID: Sessions present via native; no crashes if OMID missing.

Bridges: JNI & P/Invoke robust; IL2CPP builds pass; AOT safe.

DX: Editor panels for creds, inspector, debugger; Samples functional.

Footprint: Runtime package small (<100KB); vendor SDK inclusion controlled by publisher.

Privacy: Telemetry sanitized; BYO secrets never leave device; no S2S by default.


### iOS BYO Mediation SDK Fixes & Enhancements:

P0 fixes & additions (ship these first)
1) Concurrency & main-thread discipline

Issue: @MainActor on the entire MediationSDK makes all calls hop to main; async awaits will return to main, risking jank.

Action:

Remove class-level @MainActor. Use:

Background actors / Task.detached for config fetch, auction, adapter loads, telemetry flushes.

MainActor.run only for: view presentation, event callbacks to the app, and UI state updates.

Acceptance:

Instruments shows no >16ms blocks on main during load/show.

Unit test that loadAd performs network/disk work off main (assert Thread.isMainThread == false in injected hooks).

2) Deadline-driven adapter orchestration (BYO correctness)

Add:

Per-placement timeout budget (e.g., 600–1200ms) and per-adapter timeouts (e.g., 120–200ms).

Hedged requests (launch a backup at p95) and partial aggregation (don’t stall the whole placement on one slow adapter).

Normalized NoBid taxonomy: no_fill, timeout, network_error, status_4xx, status_5xx, below_floor, error.

CircuitBreaker on each adapter path; protect load calls.

Acceptance:

p99 load never exceeds placement timeout due to one slow adapter.

Offline conformance tests exercise 204 no_fill, 5xx retry→success, bad JSON→error, CB open/half-open transitions.

3) Consent propagation & ATT helper

Add:

Read/write IAB TCF v2 keys (IABTCF_*) and US GPP from UserDefaults behind an opt-in autoReadIAB = true.

Public setConsent(…) remains the source of truth when provided.

Provide a small ATT bridge helper: return ATTrackingManager.trackingAuthorizationStatus (don’t prompt automatically).

Acceptance:

Matrix tests (GDPR×ATT×COPPA×LMT) show expected presence/absence of IDFA and correct strings in outbound metadata.

Telemetry never contains raw identifiers when consent forbids.

4) BYO credential isolation & redaction

Action:

Ensure AdapterRegistry.initializeAdapter(config:) consumes only publisher-provided keys; never send keys to your servers.

Telemetry: hard-mask any field named like key, token, secret, signature, etc. (both keys and values).

Acceptance:

Static scan confirms no credential strings leave the device.

Debug panel shows masked values only.

5) Banner & interstitial presentation safety

Add:

A presentation coordinator that finds the topmost UIViewController safely, handles rotation, modal stacks, and safe-area insets for banners.

Single-grant logic for rewarded; prevent double callbacks.

Acceptance:

Rotation/resizing tests; multiple rapid load/show cycles; no view orphaning or layout leaks.

Rewarded callback fires exactly once.

6) Open Measurement (OM SDK) stance

BYO best-practice: if vendors render their own views, they attach OMID sessions (good). If you ever render server-mediated HTML (OpenRTB markup) inside your own web view, integrate OM SDK and manage sessions.

Acceptance:

If you render your own creatives: OMID integrated & verified with a cert partner.

If all renders are vendor SDKs: document that OMID is vendor-managed.

7) Offline conformance tests (no real network)

Add:

URLProtocol mocks for AuctionClient and adapters; golden fixtures for success/no-fill/4xx/5xx/malformed.

Latency injection and timeout tests.

Acceptance:

CI runs the matrix green; taxonomy mapping conforms across all branches.

8) Size & performance budgets

Budgets:

SDK binary: < 500 KB added to app (excluding vendor SDKs).

p95 load: ≤ 300 ms (cached) / ≤ 800 ms (network).

Acceptance:

CI job that archives a tiny host app, diffs size, and fails on regressions.

XCTest perf targets for p95.

P1 polish (soon after)

Hedged auctions vs S2S: your AuctionClient is good; default client API key empty (server-auth only), enable publisher override when needed.

Ad cache with TTL: add a small in-memory cache per placement with expiry to support isAdReady.

Paid event normalization: surface standardized OnPaidEvent(currency, precision, micros). **Unity Note:** Deferred until Managed Demand toggle; Unity runtime currently logs impressions via `OnAdEvent` only.

Debug Panel: show last N sanitized traces (placement, adapter, outcome, latency).

P2 nice-to-have

Config-as-Code export/import for placements (JSON) signed locally; helps large studios.

“Cryptographic transparency” hooks: include per-auction proof IDs when you enable them server-side (doesn’t change BYO runtime).

Sample apps: minimal SwiftUI demo with interstitial/rewarded/banner, plus test toggles.

Specific code-level nits (quick wins)

Narrow @MainActor: keep it on UI-facing methods only (e.g., show*), not the whole class.

TelemetryCollector: confirm it runs on a background queue; enforce max batch size & interval; add jitter to avoid thundering herds.

AuctionClient: ensure timeoutIntervalForRequest matches placement budget; map 429→status_429, >=500→status_5xx, 204→no_fill (you already do most of this).

ConsentManager: implement autoReadIAB gated reads and explicit setters overriding auto-read.

AdapterRegistry: assert supportsAdType before calling load; wrap calls in CB; normalize errors to taxonomy.

Redaction: centralize in a small TelemetrySanitizer and call it on every event.

BYO contract (publisher-facing)

Publisher brings: network accounts, app/site registration, placements, network SDKs (optional but typical), credentials configured in app (or via your console → device injection only).

You provide: orchestration, reliability (timeouts/hedges/CB), unified callbacks/metrics, consent plumbing, optional S2S control plane (no secrets), and a debugger.

Acceptance checklist (iOS BYO readiness)

Init/consent OK on device; matrix tests pass.

Load/Show for interstitial, rewarded, banner; single-grant; rotation safe.

Deadline & hedging enforced; p99 under budget; CB prevents cascades.

Credentials never leave device; telemetry redacted; logs sanitized.

Offline test suite green; size/perf budgets held.

No OMID needed unless you render your own creatives.



### Android TV and TVOS BYO Mediation SDK Fixes & Enhancements:

What’s already strong
Android TV (/android-tv/android-tv/...)

S2S only: AuctionClient.kt builds request → posts JSON to /rtb/bid; no embedded network credentials in SDK (✅ BYO-friendly).

Video pipeline: VideoRenderer.kt + PlayerView (Media3/ExoPlayer) is the right call for CTV.

Beacon plumbing: render/Beacon.kt fires tracking URLs async with OkHttp (good).

Config scaffolding: config/ConfigManager.kt with local cache and optional Ed25519 comment shows the right intent.

tvOS (/tvos/Sources/CTVSDK/...)

S2S only: AuctionClient.swift builds a BYO-safe request and returns AuctionWin.

Video pipeline: AVPlayer + AVPlayerViewController in InterstitialAd.swift (simple & solid).

Consent holder: Consent.swift + ConsentManager persist basic GDPR/US privacy flags.

Beacon: simple async GET in Beacon.swift (works fine for tracking endpoints).

BYO-only gaps to close (pre-GA)
Common (Android TV + tvOS)

OTA safety & kill-switch

Signed remote config (Ed25519) + staged rollout (%1 → 5 → 25 → 100) with auto-rollback on SLO breach.

Global kill-switch (per SDK, per placement).

Unified error taxonomy & timeouts

Normalize no_fill, timeout, network_error, status_XXX, error across platforms.

VAST/Tracking completeness

Fire impression + start + firstQuartile + midpoint + thirdQuartile + complete, plus mute/unmute, pause/resume, fullscreen, close as provided by auction response.

Ad caching + TTL + readiness

Cache by placementId, enforce TTL from win.ttlSeconds, surface isReady(), consume on show.

Observability

Emit counters/timers: request → response latency (p50/p95/p99), timeout count, error counts, success/fill, play start/complete, tracker send failures.

Privacy

Carry IAB TCF v2.2 / US GPP (string passthrough from host) + COPPA flag; never infer on device.

Device & context hints (CTV)

Include tv make/model if available, screen size, connection type (ethernet/wifi), OS version; make this a stable schema in the request.

Pinpoint edits (Android TV)

Paths relative to: /android-tv/android-tv/src/main/kotlin/com/rivalapexmediation/ctv

1) OTA config & kill-switch

File: config/ConfigManager.kt

Add: features.killSwitch: Boolean, features.disableShow: Boolean, placements[placementId].enabledNetworks, placements[placementId].ttlSeconds, placements[placementId].refreshSec.

Verify signatures: implement Ed25519 verification:

Accept x-config-sig: base64(ed25519(sig over SHA256(body))).

Verify with SDKConfig.configPublicKeyPem.

Staged rollout: persist last-good config; apply new version only to a random % of sessions until SLOs pass (store rollout state in SharedPreferences).

File: ApexMediation.kt

On init, if features.killSwitch, short-circuit load/show with deterministic error ("kill_switch_active").

2) Strict threading guarantees (ANR-safe)

File: ApexMediation.kt

Add (debug only):

if (BuildConfig.DEBUG) StrictMode.setThreadPolicy(
  StrictMode.ThreadPolicy.Builder().detectNetwork().detectDiskReads()
    .detectDiskWrites().penaltyLog().build()
)


Ensure all network calls are via OkHttp .enqueue (non-blocking) — already the case in AuctionClient.kt.

3) Error taxonomy & mapping

File: network/AuctionClient.kt

Map HTTP codes: 204 → no_fill, 408/504 → timeout, 5xx → status_5xx, >=400 & <500 → status_4xx (non-retry).

Expose a sealed error enum (mirrors mobile): NoFill, Timeout, Network, Status(code), Error(message).

File: ads/InterstitialAd.kt

On S2S no_fill return, bubble up onError("no_fill") (keep taxonomy identical to mobile).

4) VAST quartiles & lifecycle tracking

Files: render/VideoRenderer.kt, render/Beacon.kt, ads/InterstitialAd.kt

VideoRenderer: listen to Player.Events and media position to trigger:

start (≥2 seconds of play or ≥x% watched),

firstQuartile, midpoint, thirdQuartile, complete,

pause, resume, mute, unmute, close.

AuctionWin model (client or response DTO) must carry these tracker URLs (tracking.start, tracking.firstQuartile, …). If not present, no-op safely.

Beacon: keep non-blocking, add bounded timeouts and ignore errors (best-effort).

5) Ad cache & isReady()

File: ads/InterstitialAd.kt

Add: in-memory cache of AuctionWin keyed by placementId with expiryAt = now + ttlSeconds*1000.

Implement: isReady() checks expiry; show() consumes and clears entry.

Ensure: if expired at show time, emit onError("expired").

6) Observability metrics

Files: util/Logger.kt (or create metrics/Metrics.kt), plus touch points in ApexMediation.kt, AuctionClient.kt, InterstitialAd.kt, VideoRenderer.kt

Counters: requests_total, no_fill_total, timeouts_total, errors_total, trackers_sent_total, trackers_fail_total.

Timers: ad_load_latency_ms, video_start_latency_ms, play_time_ms.

Gate upload behind a config flag; buffer locally and flush periodically to your analytics endpoint.

7) Consent propagation

File: consent/ConsentManager.kt

Keep as the single source of truth. Ensure AuctionClient.buildConsentMap() includes gdpr_applies, tcf, us_privacy, coppa.

Do not auto-read any app storage (BYO host must set).

8) Device/context

File: network/AuctionClient.kt

Enrich buildDevice() with:

screen (width/height), connection (wifi/ethernet unknown), and tv=true.

Keep it non-PII.

Pinpoint edits (tvOS)

Paths relative to: /tvos/Sources/CTVSDK

1) ATT & attribution (privacy-first)

New file: ATT.swift

Provide an optional helper to request tracking authorization (developers call this themselves to stay BYO-compliant).

New file: Attribution.swift

Provide SKAdNetwork postback parsing & forwarding hooks (today).

Add AdAttributionKit bridge when you adopt iOS/tvOS 18+ attribution (keep behind feature flag).

2) OTA config & kill-switch

Files: ApexMediation.swift, SDKConfig.swift

Mirror Android TV: accept a signed config, stage rollout, and auto-rollback by SLO signals (local cache of last-good).

If features.killSwitch==true, refuse load() with "kill_switch_active".

3) Error taxonomy

Files: AuctionClient.swift, InterstitialAd.swift

Map the same taxonomy as Android TV (no_fill, timeout, status_4xx/5xx, network_error, error).

Ensure completion("no_fill") vs. generic error is distinct.

4) VAST quartiles & lifecycle tracking

Files: InterstitialAd.swift, New: PlayerObserver.swift

Add a KVO-backed observer on AVPlayer (timeControlStatus, item duration/position) to fire start/25/50/75/complete and UI actions (mute/unmute/pause/resume/close) via Beacon.fire(...).

Ensure all trackers are best-effort, not blocking playback.

5) Ad cache & isReady()

File: InterstitialAd.swift

Cache AuctionWin with TTL via win.ttlSeconds.

Implement isReady by TTL; clear on show.

6) Observability

Files: ApexMediation.swift, AuctionClient.swift, InterstitialAd.swift, Beacon.swift

Counters/timers equivalent to Android TV; batch to your analytics endpoint.

Gate behind config (opt-in).

7) Consent propagation

Files: Consent.swift, ApexMediation.swift, AuctionClient.swift

Ensure consent strings from host are passed verbatim (BYO-friendly); do not attempt to read system defaults automatically.

Request schema (shared BYO S2S)

Make sure the CTV request aligns with mobile (so your auction can be generic). At a minimum:

{
  "requestId": "uuid",
  "publisherId": "appId-from-SDKConfig",
  "placementId": "string",
  "adFormat": "interstitial|rewarded|preroll|midroll|postroll",
  "floorCpm": 0.0,
  "device": {
    "platform": "android_tv|tvos",
    "osVersion": "string",
    "screen": {"w": 1920, "h": 1080},
    "connection": "wifi|ethernet|unknown",
    "ifa": null
  },
  "user": {"coppa": false},
  "consent": {"gdpr_applies":null, "tcf":null, "us_privacy":null, "coppa":null},
  "features": {"tv": true}
}


Response should include:

win: creativeUrl (mp4/m3u8) and tracking map:

impression, start, firstQuartile, midpoint, thirdQuartile, complete, mute, unmute, pause, resume, close, click.

Acceptance criteria (BYO GA)

Reliability: crash-free ≥ 99.9%; no main-thread network/disk (Android TV debug guard on).

Performance: p95 ad load ≤ 800 ms (network path); video start within 1–2 s of show().

Consent: strings propagated 100% as provided; no auto-collection.

Tracking: ≥ 99.5% beacon send success (best-effort, retries capped).

Cache/readiness: isReady() true/false correctness covered in unit tests; TTL respected.

Error taxonomy: consistent strings across Android TV / tvOS and with mobile SDKs.

Kill-switch: remote flip halts serving within one cold start; hot-apply gated.

Observability: adapter-agnostic metrics posted; no PII in logs.

BYO vs. Managed-demand notes

Your current CTV SDKs are already BYO-pure (no embedded network credentials). Keep it that way.

All the additions above are additive and won’t block a later managed demand mode:

You’ll simply enrich the S2S response with exchange DSP results and preserve the same playback/trackers path.

Keep adapter-free on device for TV to minimize footprint/risk.

Quick checklist (to drive PRs)

 Android TV: StrictMode in ApexMediation.kt (debug).

 Android TV: Signed remote config + kill-switch in ConfigManager.kt; staged rollout cache.

 Android TV: Error taxonomy mapping in AuctionClient.kt; propagate to InterstitialAd.kt.

 Android TV: VAST quartiles in VideoRenderer.kt; tracker URLs on AuctionWin.

 Android TV: Cache + TTL + isReady() + consume on show in InterstitialAd.kt.

 Android TV: Metrics counters/timers (new metrics/), emit from AuctionClient.kt, InterstitialAd.kt, VideoRenderer.kt.

 tvOS: ATT helper (new), SKAN/AAK bridge (new).

 tvOS: Signed config + kill-switch in ApexMediation.swift; staged rollout.

 tvOS: Error taxonomy in AuctionClient.swift + InterstitialAd.swift.

 tvOS: VAST quartiles via PlayerObserver.swift; fire trackers via Beacon.swift.

 tvOS: Cache + TTL + isReady() + consume on show in InterstitialAd.swift.

 tvOS: Metrics counters/timers emitted from ApexMediation.swift, AuctionClient.swift, InterstitialAd.swift.