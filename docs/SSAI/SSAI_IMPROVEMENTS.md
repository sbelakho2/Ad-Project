# SSAI (Server-Side Ad Insertion) Improvements

> **Document Version:** 1.0.0  
> **Last Updated:** December 15, 2025  
> **Status:** Implementation Roadmap  
> **Scope:** Web SDK + CTV SDKs (tvOS + Android TV) + Backend Services  
> **Constraint:** Augmentative only - no breaking changes to existing functionality

---

## Table of Contents

1. [Overview](#overview)
2. [Session & Manifest Service](#1-session--manifest-service)
3. [Break Signaling & Pod Planning](#2-break-signaling--pod-planning)
4. [Pod Decisioning](#3-pod-decisioning)
5. [Stitching & Packaging](#4-stitching--packaging)
6. [Tracking & Reconciliation](#5-tracking--reconciliation)
7. [Operability & Reliability](#6-operability--reliability)
8. [CI E2E Test Matrix](#ci-e2e-test-matrix)
9. [Implementation Checklist](#implementation-checklist)

---

## Overview

This document defines the complete SSAI implementation roadmap for Apex Mediation, covering HLS + DASH protocols, TS + CMAF containers, Live + VOD modes across Web and TV platforms.

### Target Platforms

| Platform | SDK Location | Language |
|----------|--------------|----------|
| Web | `/sdk/web/` | TypeScript |
| tvOS (Apple TV) | `/sdk/ctv/tvos/` | Swift |
| Android TV | `/sdk/ctv/android-tv/` | Kotlin |
| Backend Services | `/backend/src/services/` | TypeScript |

### Protocol/Container Matrix

| Mode | Container | HLS Runners | DASH Runners |
|------|-----------|-------------|--------------|
| Live | TS | tvOS AVPlayer, Web hls.js, Android TV ExoPlayer | Web Shaka, Android TV ExoPlayer |
| Live | CMAF/fMP4 | tvOS AVPlayer, Web hls.js, Android TV ExoPlayer | Web Shaka, Android TV ExoPlayer |
| VOD | TS | tvOS AVPlayer, Web hls.js, Android TV ExoPlayer | Web Shaka, Android TV ExoPlayer |
| VOD | CMAF/fMP4 | tvOS AVPlayer, Web hls.js, Android TV ExoPlayer | Web Shaka, Android TV ExoPlayer |

---

## 1. Session & Manifest Service

> HLS + DASH "front door" for Live + VOD, TS + CMAF

### 1.1 Session Model

| Requirement | Description | SDK Targets |
|-------------|-------------|-------------|
| **SESSION-001** | `session_id` scoped to tenant + channel/asset + device capability profile + consent/policy snapshot | All SDKs |
| **SESSION-002** | TTL with refresh-on-access for Live streams | Backend + SDKs |
| **SESSION-003** | Bounded TTL for VOD (longer but still limited) | Backend + SDKs |
| **SESSION-004** | Idempotency policy to avoid session storms on player retries | Backend |

### 1.2 Playlist/MPD Serving - HLS

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **HLS-001** | Master + media playlists RFC 8216 compliant | IETF Datatracker |
| **HLS-002** | Live sliding window: `media sequence` monotonic, increments correctly when segments slide out | Apple Developer |
| **HLS-003** | Live playlist must NOT include `EXT-X-PLAYLIST-TYPE` | HLSAnalyzer |
| **HLS-004** | Don't shrink window too aggressively (avoid discontinuity sliding drift) | GitHub hls.js |
| **HLS-005** | VOD/Event: MUST end with `EXT-X-ENDLIST` | HLSAnalyzer |
| **HLS-006** | Every segment has nonzero `EXTINF`; `targetduration` correct | HLSAnalyzer |
| **HLS-007** | Use `EXT-X-DISCONTINUITY` on encoding/timestamp transitions | Tech Invite |
| **HLS-008** | `EXT-X-DISCONTINUITY-SEQUENCE` monotonic and consistent with discontinuity count | Tech Invite |

### 1.3 Playlist/MPD Serving - DASH

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **DASH-001** | Support single-period origin and multi-period SSAI output | Google Help |
| **DASH-002** | Convert single-period to multi-period on the fly | Google Help |
| **DASH-003** | MPD updates for Live: stable UTCTiming strategy | DASH-IF |
| **DASH-004** | Consistent segment timeline evolution | DASH-IF |

### 1.4 TS + CMAF Dual-Path

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **CONTAINER-001** | TS segments (.ts) and CMAF/fMP4 segments (.m4s) supported in parallel | DASH Industry Forum |
| **CONTAINER-002** | Enforce correct MIME/file conventions | DASH Industry Forum |

### 1.5 Caching/CDN Safety

| Requirement | Description |
|-------------|-------------|
| **CACHE-001** | Session manifests must be `private`/`no-store` (or very short TTL + tokenized URLs + correct `Vary`) |
| **CACHE-002** | Prevent cross-user manifest poisoning |
| **CACHE-003** | Segments can be cacheable with stable URLs |

### 1.6 Security

| Requirement | Description |
|-------------|-------------|
| **SEC-001** | Signed session token |
| **SEC-002** | Anti-tamper validation |
| **SEC-003** | Rate limits per tenant/session/IP |
| **SEC-004** | SSRF-safe URL fetcher for wrapper/creative/tracking URLs |

### 1.7 Quality Degraders (Must Prevent)

- Wrong cache headers → wrong ads served to wrong viewers
- Live playlist window slides breaking discontinuity accounting (hls.js warnings, drift)
- VOD playlists missing ENDLIST
- MPD period transitions cause player "reset" if inconsistent

### 1.8 E2E Tests - Session & Manifest

| Test Suite | Assertions |
|------------|------------|
| **HLS Live Sliding Window Compliance** | `media sequence` never decreases; increments exactly with removals; no `EXT-X-PLAYLIST-TYPE` in live; discontinuity sequence monotonic |
| **HLS VOD Correctness** | `ENDLIST` present; every `EXTINF > 0`; `targetduration` consistent |
| **Player Matrix** | Safari AVPlayer + hls.js (HLS); Shaka + dash.js (DASH); 30-60 min live stream with repeated breaks; no fatal errors; no runaway buffering |
| **Cache Poisoning** | Prove session manifests not cached publicly |
| **Retry Storm** | Players requesting manifest every 250ms during network flaps → service stable, rate limiting works, session count doesn't explode |

---

## 2. Break Signaling & Pod Planning

> SCTE-35 + schedules for ad opportunity signaling

### 2.1 Signal Ingestion

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **SIGNAL-001** | Live: ingest SCTE-35 cues (`time_signal`/`splice_insert` patterns) from upstream | Bitmovin |
| **SIGNAL-002** | HLS: represent cues via `EXT-X-DATERANGE`, alongside `EXT-X-PROGRAM-DATE-TIME` | AWS Documentation |
| **SIGNAL-003** | `DURATION`/`PLANNED-DURATION`/`END-DATE` may be optional | AWS Documentation |
| **SIGNAL-004** | DASH: represent cues using MPEG-DASH Events/EventStream schemes for SCTE-35 signaling | DVB |
| **SIGNAL-005** | VOD: support embedded markers from live-to-VOD harvest pass-through | AWS Documentation |

### 2.2 Pod Planning

| Requirement | Description |
|-------------|-------------|
| **POD-001** | Normalize every break into: `break_id`, `pod_id`, start time (wall clock + media time), planned duration, slot list |
| **POD-002** | Handle missing duration and "END-ON-NEXT" / end cue style flows |
| **POD-003** | Enforce splice point alignment: cue must land on IDR/keyframe, or align at nearest safe boundary |
| **POD-004** | If cue doesn't land on keyframe: encoder/origin must insert keyframes or splice at safe boundary |

### 2.3 De-dup & Drift

| Requirement | Description |
|-------------|-------------|
| **DEDUP-001** | De-dup duplicate cues (same cue id/time window) |
| **DRIFT-001** | Handle cues arriving late/early |
| **DRIFT-002** | Define behavior for drift: partial fill vs slate |

### 2.4 Quality Degraders (Must Prevent)

- Cue lands mid-GOP → black frame/glitch unless encoder inserts keyframes
- DATERANGE IDs inconsistent across updates → clients misinterpret
- DASH: multiple events in same Period treated differently; some SSAI stacks only act on first Event per Period

### 2.5 E2E Tests - Break Signaling

| Test Suite | Assertions |
|------------|------------|
| **Cue Integrity - Duplicate** | Duplicate cue injection → only one break created |
| **Cue Integrity - Out-of-Order** | Out-of-order cue timestamps → system stabilizes, consistent pod start/end |
| **Keyframe Alignment** | Cue not at IDR → system aligns splice safely OR rejects with clear error and falls back to slate |
| **DATERANGE Optionality** | Cue with no duration → manifests omit DURATION; system runs and ends break on end cue |
| **DASH Event Behavior** | Multiple SCTE-35 Events in one Period → deterministic, documented behavior; test "only first event used" pattern |

---

## 3. Pod Decisioning

> BYO demand with deadlines, separation, pacing

### 3.1 Decision SLA

| Requirement | Description |
|-------------|-------------|
| **SLA-001** | Hard deadline: always return pod plan before break start (or before each slot boundary for live) |
| **SLA-002** | Safe mode: reduce partner fanout |
| **SLA-003** | Safe mode: force slate after deadline |
| **SLA-004** | Safe mode: freeze config changes during incident |

### 3.2 Constraints

| Requirement | Description |
|-------------|-------------|
| **CONST-001** | Competitive separation (advertiser/category/creative repetition) |
| **CONST-002** | Frequency caps per session |
| **CONST-003** | Frequency caps per household/device when allowed |
| **CONST-004** | Frequency caps in contextual/no-ID mode |
| **CONST-005** | Pacing/throttles per partner and per tenant |
| **CONST-006** | Circuit breakers with cooldown (timeout rate, error rate thresholds) |
| **CONST-007** | Determinism: same input + config version + health state → same candidate ordering |

### 3.3 Format Compatibility Contract

| Requirement | Description |
|-------------|-------------|
| **COMPAT-001** | Creative must match delivery profile: TS vs CMAF, HLS vs DASH, codecs, audio layout, captions policy |
| **COMPAT-002** | Reject incompatible creative early and explain why |

### 3.4 Quality Degraders (Must Prevent)

- Single slow partner poisons pod latency → need circuit breakers
- Repeated ads in pod → separation/caps missing
- "Won't play on Roku / Safari" → format mismatch (TS vs fMP4) and codec ladders

### 3.5 E2E Tests - Pod Decisioning

| Test Suite | Assertions |
|------------|------------|
| **Latency Poison** | Partner returns in 2s 30% of time → breaker trips; p95 pod decision within budget |
| **Separation + Cap Torture** | 10 pods in same session → no illegal repeats; caps respected under low fill |
| **Compatibility Rejection** | HLS/TS-only creatives into DASH/CMAF session → rejected with explicit reason, fallback used |

---

## 4. Stitching & Packaging

> Playback-proof engine for Live + VOD, TS + CMAF

### 4.1 Stitching Models

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **STITCH-001** | Support Replacement (swap content segments with ad segments) | DASH Industry Forum |
| **STITCH-002** | Support Insertion (insert ads into timeline) | DASH Industry Forum |

### 4.2 HLS Specifics

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **HLS-STITCH-001** | Use `EXT-X-DISCONTINUITY` at encoding transitions | Tech Invite |
| **HLS-STITCH-002** | Keep discontinuity sequence accounting consistent across sliding windows | Tech Invite |
| **HLS-STITCH-003** | DATERANGE for cues works with program date-time and sliding window behavior | AWS Documentation |

### 4.3 DASH Specifics

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **DASH-STITCH-001** | Support multi-period outputs | Google Help |
| **DASH-STITCH-002** | If origin is single-period, rewrite accordingly | Google Help |
| **DASH-STITCH-003** | Use EventStream scheme rules for SCTE-35 per DVB-DASH recommendations | DVB |

### 4.4 TS + CMAF Correctness

| Requirement | Description | Reference |
|-------------|-------------|-----------|
| **SEGMENT-001** | Maintain consistent fragment/segment durations | Unified Streaming |
| **SEGMENT-002** | Align audio/video fragments at splice boundaries | Unified Streaming |
| **SEGMENT-003** | One GOP per fragment where possible (improves splicing/clipping) | Unified Streaming |

### 4.5 DRM Policy

| Requirement | Description |
|-------------|-------------|
| **DRM-001** | Declare and enforce: clear vs encrypted ads |
| **DRM-002** | Key continuity at boundaries |
| **DRM-003** | Define whether ads can be unencrypted in encrypted content streams |

### 4.6 Audio/Captions

| Requirement | Description |
|-------------|-------------|
| **AUDIO-001** | Loudness policy (at minimum detect and flag extremes) |
| **CAPTION-001** | Caption continuity policy (content captions across breaks) |
| **CAPTION-002** | Ad captions optional but must not crash players |

### 4.7 Quality Degraders (Must Prevent)

- A/V not aligned at splice boundary → "audio pops" / drift
- Not enough keyframes at cue boundaries → glitches
- DASH period transitions mishandled → player rebuffer/reset

### 4.8 E2E Tests - Stitching

| Test Suite | Assertions |
|------------|------------|
| **Splice-Boundary Torture (Live)** | 100+ breaks over 2 hours; no fatal errors; no infinite buffering; no A/V drift accumulation |
| **Sliding-Window Discontinuity** | Discontinuities near window edges → cc/discontinuity sequence consistent; hls.js no drift warnings |
| **TS↔CMAF Compatibility** | Same channel as TS/HLS and CMAF/HLS; same for DASH; correct packaging; wrong-format creatives rejected |
| **Audio Alignment** | Audio/video segment boundaries align at splice points (no overlapping timestamps) |
| **DRM Boundary** | Encrypted content with ad insertion per policy; playback works; keys transition correctly |

---

## 5. Tracking, Reconciliation, and Proof

> Server-side measurement that withstands disputes

### 5.1 Event Model

| Requirement | Description |
|-------------|-------------|
| **EVENT-001** | Impression/start/quartiles/complete/error, per slot |
| **EVENT-002** | Exactly-once semantics with dedupe keys: `(session_id, pod_id, slot_id, event_type, creative_id)` |
| **EVENT-003** | Handle missing callbacks (SSAI inference) vs hybrid callbacks |

### 5.2 VOD vs Live Reconciliation

| Requirement | Description |
|-------------|-------------|
| **RECON-001** | VOD: deterministic timeline → exact quartiles |
| **RECON-002** | Live: tolerate drift and variable segment durations; still must be stable |

### 5.3 Signed Evidence

| Requirement | Description |
|-------------|-------------|
| **EVIDENCE-001** | Produce signed "audit bundle": decision trace + manifest_version + cue details + chosen creatives + event timeline |

### 5.4 Rate Limiting

| Requirement | Description |
|-------------|-------------|
| **RATE-001** | Tracking endpoints rate-limited |
| **RATE-002** | Backoff and jitter to prevent event storms during outages |

### 5.5 Quality Degraders (Must Prevent)

- Double beacons from retries → inflation
- Clock drift and live latency → quartile miscounts
- Partner disputes → no proof bundle

### 5.6 E2E Tests - Tracking

| Test Suite | Assertions |
|------------|------------|
| **Replay Storm** | Clients retrying tracking beacons 10x → backend records once |
| **Clock Skew** | ±60s skew and rebuffer events → quartile computation consistent; errors classified |
| **Dispute Bundle** | Export bundle for window → signature valid; totals match rollups |

---

## 6. Operability & Reliability

> World-class "complaint killers"

### 6.1 Explainability

| Requirement | Description |
|-------------|-------------|
| **EXPLAIN-001** | "Explain pod/break" shows: cue ingestion → pod plan → partner calls/timings → creative accept/reject reasons → stitched output → tracking outcomes |
| **EXPLAIN-002** | "Explain no-fill" returns actionable causes (timeout vs policy vs incompatibility vs low demand) |

### 6.2 Control Plane

| Requirement | Description |
|-------------|-------------|
| **CTRL-001** | Kill switches: per tenant, channel, region, partner, format (TS vs CMAF), protocol (HLS vs DASH) |
| **CTRL-002** | Rollouts: canary by geography/device class/player family |
| **CTRL-003** | Instant rollback capability |
| **CTRL-004** | Safe mode: reduce fanout; stricter deadlines; enforce slate; freeze changes |

### 6.3 SSAI SLIs & Alerting

| Requirement | Description |
|-------------|-------------|
| **SLI-001** | Manifest generation error rate (HLS & DASH) |
| **SLI-002** | Segment fetch error rate |
| **SLI-003** | Cue ingestion lag/missing cues |
| **SLI-004** | Pod start latency vs deadline |
| **SLI-005** | Stall/rebuffer proxy (player-side metrics if available, otherwise infer from request patterns) |
| **SLI-006** | Partner timeout/error rates + breaker trips |

### 6.4 Resilience

| Requirement | Description |
|-------------|-------------|
| **RESIL-001** | Backpressure everywhere (queues, bounded retries) |
| **RESIL-002** | Multi-zone strategy for control plane and manifest service (if claiming high availability) |
| **RESIL-003** | Graceful degradation always available (slate insertion) |

### 6.5 Quality Degraders (Must Prevent)

- "We didn't know it broke" → missing alert SLIs
- "Debugging takes days" → no explain view; no signed trace
- Partner outage takes down everything → lack of breakers and safe mode

### 6.6 E2E Tests - Operability

| Test Suite | Assertions |
|------------|------------|
| **Chaos Test (Dependency Failures)** | Randomly fail: cue ingestion, ad decision, manifest render, segment origin fetch → playback continues with slate; alerts trigger with correct root cause |
| **Partner Outage Regression** | Partner 100% timeout → breaker triggers within N calls; fill degrades gracefully without deadline misses |
| **Rollback Survival** | Deploy "bad" config causing incompatibility rejects → rollback restores normal fill; audit log records diff and affected cohorts |

---

## CI E2E Test Matrix

### A) Core Protocol/Container/Mode Matrix (8 Variants)

| Mode | Container | HLS Runner Set | DASH Runner Set |
|------|-----------|----------------|-----------------|
| Live | TS | tvOS AVPlayer + Web hls.js + Android TV ExoPlayer(HLS) | Web Shaka + Android TV ExoPlayer(DASH) |
| Live | CMAF/fMP4 | tvOS AVPlayer + Web hls.js + Android TV ExoPlayer(HLS) | Web Shaka + Android TV ExoPlayer(DASH) |
| VOD | TS | tvOS AVPlayer + Web hls.js + Android TV ExoPlayer(HLS) | Web Shaka + Android TV ExoPlayer(DASH) |
| VOD | CMAF/fMP4 | tvOS AVPlayer + Web hls.js + Android TV ExoPlayer(HLS) | Web Shaka + Android TV ExoPlayer(DASH) |

### B) Marker/Signaling Matrix

Run each of 8 variants with:

| Marker Source | Marker Carriage |
|---------------|-----------------|
| SCTE-35 marker-driven | HLS: EXT-X-DATERANGE + PDT |
| Schedule-driven (FAST style) | DASH: EventStream / emsg |

### C) Ad Break Shape Matrix

| Pod Scenario | Description |
|--------------|-------------|
| Single-slot pod | 1×30s |
| Multi-slot pod | 4×15s |
| Mixed durations | 10s + 20s + 15s |
| Unknown duration start | End cue arrives later (live) |
| Late cue arrival | Cue arrives close to break start |

### D) Creative/Format Compatibility Matrix

| Scenario | Expected Behavior |
|----------|-------------------|
| Perfect match (same codec ladder/audio) | Plays normally |
| Codec mismatch | Reject + fallback |
| Audio layout mismatch (stereo vs 5.1) | Reject or condition per policy |
| Segment duration mismatch | Stitch without fatal, or reject |
| Keyframe misalignment stress | Splice safely or slate |

### E) Fault/Chaos Matrix

| Category | Fault |
|----------|-------|
| Origin/CDN | Manifest fetch 5xx burst |
| Origin/CDN | Segment 404 mid-ad |
| Origin/CDN | Slow origin (high RTT) |
| Origin/CDN | CDN cache churn (manifests must remain private/no-store) |
| Partner/Demand | Partner timeout 30% of calls (circuit breaker must trip) |
| Partner/Demand | Partner returns duplicates (separation rules enforced) |
| Partner/Demand | Partner returns incompatible creative (reject reason logged) |
| Network | Packet loss + jitter profile (safe mode triggers; no playback stall) |
| Network | Connectivity flapping (no retry storm) |

### F) DRM/Encryption Matrix (Extended)

| Scenario | Status |
|----------|--------|
| Clear content + clear ads | Required |
| DRM content + clear ads (if allowed) | Optional |
| DRM content + DRM ads (same scheme) | Optional |

### G) Minimum Pass Criteria Per Run

Every test run must assert:

- [ ] Playback completes (or reaches defined runtime) with no fatal player errors
- [ ] No deadline misses at ad breaks (or misses stay under strict threshold)
- [ ] Manifests validate (syntax + sequencing rules)
- [ ] "Explain pod" + signed log exists for sampled requests
- [ ] No retry storms; rate limits respected; circuit breaker behavior correct

### H) Long-Run Soak Tests

6–12 hour runs with:
- Frequent breaks
- Network jitter
- Origin 5xx bursts
- CDN cache churn

Assert:
- No manifest rule violations
- No fatal playback errors
- Deadline miss rate under strict threshold
- Discontinuity accounting stable

---

## Implementation Checklist

### Phase 1: Backend SSAI Service Foundation

- [ ] **1.1** Create `/backend/src/services/ssai/` directory structure
- [ ] **1.2** Implement `SessionService` (SESSION-001 to SESSION-004)
- [ ] **1.3** Implement `ManifestService` base class
- [ ] **1.4** Implement `HLSManifestGenerator` (HLS-001 to HLS-008)
- [ ] **1.5** Implement `DASHManifestGenerator` (DASH-001 to DASH-004)
- [ ] **1.6** Implement `ContainerHandler` for TS/CMAF dual-path
- [ ] **1.7** Implement cache headers and CDN safety (CACHE-001 to CACHE-003)
- [ ] **1.8** Implement security layer (SEC-001 to SEC-004)

### Phase 2: Break Signaling & Pod Planning

- [ ] **2.1** Implement `SCTE35Parser` (SIGNAL-001)
- [ ] **2.2** Implement `HLSCueHandler` with DATERANGE support (SIGNAL-002, SIGNAL-003)
- [ ] **2.3** Implement `DASHEventHandler` with EventStream support (SIGNAL-004)
- [ ] **2.4** Implement `VODMarkerProcessor` (SIGNAL-005)
- [ ] **2.5** Implement `PodPlanner` with normalization (POD-001 to POD-004)
- [ ] **2.6** Implement cue deduplication (DEDUP-001)
- [ ] **2.7** Implement drift handling (DRIFT-001, DRIFT-002)

### Phase 3: Pod Decisioning

- [ ] **3.1** Implement `DecisionSLAEnforcer` (SLA-001 to SLA-004)
- [ ] **3.2** Implement `CompetitiveSeparation` (CONST-001)
- [ ] **3.3** Implement `FrequencyCapManager` (CONST-002 to CONST-004)
- [ ] **3.4** Implement `PacingThrottler` (CONST-005)
- [ ] **3.5** Enhance circuit breaker for SSAI (CONST-006)
- [ ] **3.6** Implement deterministic candidate ordering (CONST-007)
- [ ] **3.7** Implement `FormatCompatibilityValidator` (COMPAT-001, COMPAT-002)

### Phase 4: Stitching & Packaging

- [ ] **4.1** Implement `StitchingEngine` with replacement/insertion modes (STITCH-001, STITCH-002)
- [ ] **4.2** Implement HLS discontinuity handling (HLS-STITCH-001 to HLS-STITCH-003)
- [ ] **4.3** Implement DASH multi-period rewriting (DASH-STITCH-001 to DASH-STITCH-003)
- [ ] **4.4** Implement segment alignment (SEGMENT-001 to SEGMENT-003)
- [ ] **4.5** Implement DRM policy enforcement (DRM-001 to DRM-003)
- [ ] **4.6** Implement audio loudness detection (AUDIO-001)
- [ ] **4.7** Implement caption continuity (CAPTION-001, CAPTION-002)

### Phase 5: Tracking & Reconciliation

- [ ] **5.1** Implement SSAI event model (EVENT-001 to EVENT-003)
- [ ] **5.2** Implement exactly-once dedupe layer
- [ ] **5.3** Implement VOD quartile calculator (RECON-001)
- [ ] **5.4** Implement Live reconciliation with drift tolerance (RECON-002)
- [ ] **5.5** Implement signed audit bundle generation (EVIDENCE-001)
- [ ] **5.6** Implement tracking rate limiting (RATE-001, RATE-002)

### Phase 6: Operability & Reliability

- [ ] **6.1** Implement "Explain pod/break" endpoint (EXPLAIN-001)
- [ ] **6.2** Implement "Explain no-fill" endpoint (EXPLAIN-002)
- [ ] **6.3** Implement kill switch infrastructure (CTRL-001)
- [ ] **6.4** Implement canary rollout system (CTRL-002, CTRL-003)
- [ ] **6.5** Implement safe mode (CTRL-004)
- [ ] **6.6** Implement SSAI SLI metrics (SLI-001 to SLI-006)
- [ ] **6.7** Implement backpressure and bounded retries (RESIL-001)
- [ ] **6.8** Implement graceful degradation with slate (RESIL-003)

### Phase 7: Web SDK Integration

- [ ] **7.1** Add SSAI session types to `/sdk/web/src/types.ts`
- [ ] **7.2** Implement `SSAIClient` in `/sdk/web/src/ssai/`
- [ ] **7.3** Add HLS manifest parsing with hls.js integration
- [ ] **7.4** Add DASH manifest parsing with Shaka integration
- [ ] **7.5** Implement tracking beacon firing
- [ ] **7.6** Add SSAI events to SDK event emitter

### Phase 8: tvOS SDK Integration

- [ ] **8.1** Add SSAI types to `/sdk/ctv/tvos/Sources/CTVSDK/`
- [ ] **8.2** Implement `SSAISessionManager.swift`
- [ ] **8.3** Implement AVPlayer manifest interception
- [ ] **8.4** Implement SCTE-35 cue detection
- [ ] **8.5** Implement tracking beacon firing
- [ ] **8.6** Add SSAI metrics to MetricsRecorder

### Phase 9: Android TV SDK Integration

- [ ] **9.1** Add SSAI types to `/sdk/ctv/android-tv/`
- [ ] **9.2** Implement `SSAISessionManager.kt`
- [ ] **9.3** Implement ExoPlayer manifest interception
- [ ] **9.4** Implement SCTE-35 cue detection
- [ ] **9.5** Implement tracking beacon firing
- [ ] **9.6** Add SSAI metrics to MetricsRecorder

### Phase 10: E2E Test Suites

- [ ] **10.1** Create SSAI test harness with all 8 protocol/container variants
- [ ] **10.2** Implement marker/signaling test suite
- [ ] **10.3** Implement ad break shape test suite
- [ ] **10.4** Implement creative/format compatibility test suite
- [ ] **10.5** Implement fault/chaos test suite
- [ ] **10.6** Implement DRM test suite (if applicable)
- [ ] **10.7** Create long-run soak test automation (6-12 hours)
- [ ] **10.8** Create CI pipeline for SSAI test matrix

---

## File Structure

```
backend/src/services/ssai/
├── index.ts                    # SSAI service exports
├── session/
│   ├── SessionService.ts       # Session management
│   ├── SessionStore.ts         # Session persistence
│   └── types.ts                # Session types
├── manifest/
│   ├── ManifestService.ts      # Base manifest service
│   ├── HLSManifestGenerator.ts # HLS manifest generation
│   ├── DASHManifestGenerator.ts # DASH manifest generation
│   └── types.ts                # Manifest types
├── signaling/
│   ├── SCTE35Parser.ts         # SCTE-35 parsing
│   ├── HLSCueHandler.ts        # HLS DATERANGE handling
│   ├── DASHEventHandler.ts     # DASH EventStream handling
│   └── types.ts                # Signaling types
├── pod/
│   ├── PodPlanner.ts           # Pod planning
│   ├── DecisionSLAEnforcer.ts  # SLA enforcement
│   ├── FormatValidator.ts      # Format compatibility
│   └── types.ts                # Pod types
├── stitching/
│   ├── StitchingEngine.ts      # Stitching logic
│   ├── HLSStitcher.ts          # HLS-specific stitching
│   ├── DASHStitcher.ts         # DASH-specific stitching
│   └── types.ts                # Stitching types
├── tracking/
│   ├── EventTracker.ts         # Event tracking
│   ├── Reconciler.ts           # Reconciliation
│   ├── AuditBundleGenerator.ts # Signed audit bundles
│   └── types.ts                # Tracking types
├── operability/
│   ├── ExplainService.ts       # Explain pod/break
│   ├── KillSwitchManager.ts    # Kill switches
│   ├── SafeModeController.ts   # Safe mode
│   └── SLIMetrics.ts           # SLI metrics
└── __tests__/
    ├── session.test.ts
    ├── manifest.test.ts
    ├── signaling.test.ts
    ├── pod.test.ts
    ├── stitching.test.ts
    ├── tracking.test.ts
    └── e2e/
        ├── hls-live-ts.test.ts
        ├── hls-live-cmaf.test.ts
        ├── hls-vod-ts.test.ts
        ├── hls-vod-cmaf.test.ts
        ├── dash-live-ts.test.ts
        ├── dash-live-cmaf.test.ts
        ├── dash-vod-ts.test.ts
        └── dash-vod-cmaf.test.ts

sdk/web/src/ssai/
├── index.ts                    # SSAI client exports
├── SSAIClient.ts               # Main SSAI client
├── HLSManifestParser.ts        # HLS parsing
├── DASHManifestParser.ts       # DASH parsing
├── TrackingBeacon.ts           # Beacon firing
├── types.ts                    # SSAI types
└── __tests__/
    └── ssai.test.ts

sdk/ctv/tvos/Sources/CTVSDK/SSAI/
├── SSAISessionManager.swift    # Session management
├── ManifestInterceptor.swift   # AVPlayer interception
├── SCTE35Detector.swift        # Cue detection
├── TrackingBeacon.swift        # Beacon firing
└── SSAITypes.swift             # SSAI types

sdk/ctv/android-tv/src/main/kotlin/com/rivalapexmediation/ctv/ssai/
├── SSAISessionManager.kt       # Session management
├── ManifestInterceptor.kt      # ExoPlayer interception
├── SCTE35Detector.kt           # Cue detection
├── TrackingBeacon.kt           # Beacon firing
└── SSAITypes.kt                # SSAI types
```

---

## References

- [RFC 8216 - HTTP Live Streaming](https://tools.ietf.org/html/rfc8216)
- [DASH Industry Forum Guidelines](https://dashif.org/guidelines/)
- [SCTE-35 Digital Program Insertion Cueing](https://www.scte.org/standards/)
- [DVB-DASH Guidelines](https://dvb.org/)
- [AWS Elemental MediaTailor Documentation](https://docs.aws.amazon.com/mediatailor/)
- [Google Ad Manager DAI](https://support.google.com/admanager/answer/7295798)
- [Unified Streaming Best Practices](https://docs.unified-streaming.com/)
- [Apple HLS Authoring Specification](https://developer.apple.com/documentation/http_live_streaming)
