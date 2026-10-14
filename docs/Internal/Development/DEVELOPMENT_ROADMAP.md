# Development Roadmap — Coding First, External Testing Last

_Last updated: 2025-11-18 16:12 UTC_

> **Governance:** Every status claim in this roadmap must align with the canonical readiness record in `docs/Internal/Deployment/PROJECT_STATUS.md`. Add new work here only after the risk is logged in `docs/Internal/Development/AD_PROJECT_FILE_ANALYSIS.md`.

Document purpose: Translate COMPETITIVE_GAP_ANALYSIS insights into a concrete, engineering-first roadmap. All coding work is done before final external/sandbox integration tests. Sandbox credentials for ad networks, MMPs/SKAN, payment processors, etc., will be used only in the Final Test & Certification phase.

Status legend:
- [P0] = Critical path to parity and reliability (0–6 weeks)
- [P1] = High-value enhancements (6–12 weeks)
- [P2] = Expansion and excellence (12–20 weeks)
- [FT] = Final Test & Certification (after P0/P1 core coding is complete)

Success criteria are measurable and map to KPIs defined in COMPETITIVE_GAP_ANALYSIS.md.

---

## Guiding Principles
- Coding-first: implement features, resiliency, and observability without using external creds.
- Deterministic tests: extensive unit and integration tests with mocks, golden files, and contract fixtures.
- Safety-first ML: fraud model remains in shadow mode until evaluation targets are achieved.
- Observability and SLOs across all critical paths before any external certification.
- Cost governance: entire system (infra + autonomy/LLM) operates under a hard cap of $500/month with thresholds and graceful degradation (see docs/Internal/COST_BUDGET_POLICY.md).
- Autonomy loop: weekly planner + scoped change executor operate under cost caps and safety guardrails; dual LLM providers (Junie + ChatGPT) with cost-aware routing.

---

## P0: Reliability, Resiliency, and Observability (0–6 weeks)

1) Standardize adapter resiliency across all current bidders (AdMob, Meta, Unity, AppLovin, ironSource)
- Single-retry with jitter on transient errors (already in some adapters; make uniform)
- Simple in-memory circuit breaker per adapter instance (open after N failures for T seconds)
- NoBid reason taxonomy: timeout, network_error, status_XXX, no_fill, below_floor_price, circuit_open
- Hedged requests (optional feature flag) for slow adapters; ensure no auction stall
- Partial aggregation tolerance: accept subset of bids within deadline

2) Adapter metrics, tracing, and SLOs
- Per-adapter metrics: request count, latency p50/p95, error rate, fill rate, timeouts
- Trace spans: request build, HTTP call, parse/mapping, winner selection
- Structured logs: include request_id, adapter, attempt, circuit state
- Define SLOs and alerts: e.g., error rate <2%, p95 latency <300ms per adapter

3) Mediation debugger (MVP)
- Server-side capture of adapter request/response summaries with PII redaction
- Deterministic replay mode for auctions (golden fixtures)
- Admin API to fetch last-N auction traces for a placement

4) ML fraud pipeline foundations (shadow mode)
- Feature parity contract between training and inference; schema versioning
- Offline training job scaffolding using internal synthetic + weakly supervised labels (see ML_FRAUD_TRAINING_DATA_SOURCES.md)
- Evaluation protocol: compute AUC/PR, precision/recall at multiple thresholds; export metrics to model artifact
- Enforce shadow-mode gating when metrics are degenerate (already added) and until targets are met

5) CI enhancements
- Unit tests for adapters (request building and response mapping via fixtures)
- Contract/golden tests for mediation debugger and auction engine selection logic
- Static analysis, linting, race detector on Go code, type checks on TS

Acceptance criteria (P0)
- All adapters share common resiliency behaviors and no-bid taxonomy
- Metrics and traces visible locally; SLO doc checked in
- Mediation debugger can capture and replay at least 1 full auction with 2+ adapters
- ML stays in shadow mode automatically if metrics ineffective; training job produces metrics JSON
- CI green: adapters’ golden tests and engine tests pass

---

## P1: Optimization, SDK DX, Privacy (6–12 weeks)

6) Revenue optimization features
- Dynamic floors with rules and experiments
- eCPM decay and recency-weighted scoring
- Frequency capping & pacing per user/placement
- A/B/n experimentation with Thompson Sampling or UCB variants

7) SDK samples and integration quality
- Sample apps: Android, iOS, Web with rendering examples (banner/interstitial/rewarded)
- Integration linter: static checks for required initialization/events/consents
- Network mocks for SDK-side ad rendering and error handling

8) Privacy and compliance test matrix
- GDPR/CCPA consent propagation E2E tests (mocks)
- ATT (iOS) mode toggles respected throughout adapters and SDK
- SKAdNetwork schema plumbing in events; basic validations

9) Analytics and ingestion validation
- Deterministic ingestion tests for ClickHouse/Postgres (using dockerized local env)
- Dashboard API completeness checks; seed data + snapshots

Acceptance criteria (P1)
- Optimization primitives available via config with unit/integration tests
- SDK sample apps compile and run locally with mocks, rendering happy-path and error states
- Privacy tests cover consent/ATT/PII redaction paths; SKAN fields present in schemas
- Ingestion tests validate core analytics KPIs derivation

---

## P2: Coverage Expansion, Finance Ops, and Scale (12–20 weeks)

10) Adapter expansion and certification readiness
- Implement 7+ additional adapters to reach ≥12 total (shortlist: Mintegral, Chartboost, Vungle, AdColony, Pangle, Snap, TikTok)
- Uniform request/response contracts with fixtures and mocks; no external creds yet

11) Revenue reconciliation & payments foundations
- Event contracts for revenue reports and discrepancy analysis
- Internal reconciliation jobs (mock providers); alerting on deviations
- Payment processor integration stubs with contract tests (Adyen/Adyen mock APIs)

12) Cohort/LTV analytics dashboards
- Queries and APIs for cohort retention, ARPDAU, LTV by channel/placement
- Snapshot tests for core queries

13) Load and performance testing
- Synthetic high-QPS auction load; verify scheduler and deadlines
- Latency budgets documented; results checked in

Acceptance criteria (P2)
- ≥12 adapters implemented with tests and fixtures
- Reconciliation jobs produce discrepancy reports on mock datasets
- Payments stubs pass contract tests; no secrets required
- Load test scripts and results included; bottlenecks and fixes tracked

---

## FT: Final Test & Certification (external sandboxes only after coding complete)

14) Sandbox credentials and conformance
- Obtain sandbox creds for: AdMob, Meta, Unity, AppLovin, ironSource, additional networks selected in P2
- Run conformance suites per adapter: auth, headers, schemas, response parsing, no-bid reasons
- Validate payment processor flows with sandbox APIs

15) Go/No-Go and rollout plan
- ML fraud: leave shadow mode only if evaluation meets targets (AUC ≥ 0.85, Precision ≥ 0.8 at Recall ≥ 0.9)
- SLO burn-rate review for adapters and auction engine
- Update public documentation and changelogs

Acceptance criteria (FT)
- All targeted adapters pass sandbox conformance
- Payments sandbox flows pass
- ML remains in shadow unless targets achieved; otherwise proceed with shadow + monitoring

---

## Traceability
- Derived from: docs/Internal/COMPETITIVE_GAP_ANALYSIS.md and docs/Internal/ML_FRAUD_TRAINING_DATA_SOURCES.md
- Linked context: GAPS_RESOLUTION.md addendum 2025-11-06 (Competitive Gap Plan + ML Data Sources)

## How to Use This Roadmap
- Treat sections as epics with issues per bullet.
- Keep external creds out of CI; use mocked tests until FT.
- Update this file as epics land; move items between phases only with justification.

## Change Log
| Date | Change |
| --- | --- |
| 2025-11-18 | Added FIX-10 governance banner tying roadmap claims to `FIXES.md`, `PROJECT_STATUS.md`, and `AD_PROJECT_FILE_ANALYSIS.md`. |
