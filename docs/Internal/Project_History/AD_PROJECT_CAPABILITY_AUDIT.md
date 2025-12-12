# Ad-Project Capability and Effectiveness Audit

Date: 2025-11-06
Auditor: Automated Repository Review (Junie)
Scope: Validate whether the repository can actually deliver the features documented in customer-facing and internal docs. Identify functional gaps and assess the ML anti-fraud model’s effectiveness.

---

## Executive Verdict

- Overall: Partially functional. Strong architectural coverage and significant code present across auction, bidding adapters, fraud detection (ML + rules), services, and data schemas. However, production readiness requires additional integration, verification, and operational hardening.
- Works now (with configuration and mocks):
  - Auction engine (header bidding + waterfall + hybrid), selection logic, timeouts, and quality scoring scaffolding.
  - Basic bidder adapters for AdMob, Meta, AppLovin, and Unity with HTTP call flows and request/response mapping.
  - Fraud detection module (Go) with an ML scoring pipeline, feature vector, and persistence hook (Redis), plus Node/TS controllers and repository layer to serve dashboard stats.
  - Data schemas for Postgres and ClickHouse (including fraud events, summaries, and indices), and reporting scaffolding.
- Does not fully work as advertised without more work:
  - External ad network integrations are not validated end-to-end; credentials, live endpoints, request conformance, and creative rendering aren’t fully proven.
  - Fraud ML “99.7% accuracy” is not substantiated by current training artifacts; the bundled trained model shows degenerate metrics (precision/recall 0) and is likely unsuitable for production blocking.
  - Observability, rate limiting, retries, and failure-mode handling for network adapters need hardening.
  - Some dashboards are missing front-end implementations despite existing APIs.

Bottom line: The platform can function in a controlled/dev environment, but several gaps must be closed for a production-grade, reliable, and effective system aligned with documentation claims.

---

## Evidence-Based Assessment by Subsystem

### 1) Mediation and Auctioning
- Code: backend/auction/internal/bidding/engine.go
  - Features confirmed in code:
    - Auction types: First-price, Second-price, HeaderBidding, Waterfall, Hybrid enum and branching.
    - Parallel bid collection with timeout handling and latency tracking.
    - Aggregation and best-bid selection; quality score hooks mentioned in docs; partial evidence in engine.
    - Uses Redis client (likely for configuration/state) but not a hard dependency for basic auction flow.
- Adapters (backend/auction/internal/bidders/):
  - admob.go: Constructs structured request, includes GDPR/COPPA fields, UA/IP, device attributes. Custom JSON types map to OpenRTB-like structures. Endpoint and exact payload may differ from Google’s production RTB requirements; requires validation against AdMob’s current bidding APIs. Authentication: API key header in code; real AdMob integrations often require precise auth and schemas.
  - meta.go: POSTS to Graph-like endpoint; includes consent flags and device/user/app fields; maps response to generic BidResponse. Needs validation against Meta Audience Network’s current bidding specs and app review constraints.
  - applovin.go: Talks to ms.applovin.com mediation API; maps payload fields and parses response; handles 204 no-fill; basic success parsing present.
  - unity.go: Targets Unity auction endpoint; builds payload for platform/device/tracking; parses fills; returns generic response or no-bid.
- Observations:
  - The adapters are “plausible” and structurally correct but unverified against live networks. Keys required to function are not bundled (correct). Error handling exists (status checks, NoBid reasons), but production resiliency (retry/backoff, circuit breaking, structured no-bid taxonomies) is limited.
  - No creative rendering examples in SDKs reviewed in this audit; adapters return ad_markup but end-to-end render not proven here.
- Conclusion: Auction engine is functionally solid. Adapter implementations are credible starting points but need conformance testing with sandbox credentials and real endpoints. The claim of “50+ networks” is not met in code (5 present).

### 2) Fraud Detection (ML + Rules)
- Core ML: backend/fraud/internal/ml/fraud_ml.go
  - Implements logistic regression scoring with bias, 50+ features spanning device, behavior, temporal, network, UA, historical KPIs.
  - Loads a trained model if available (trained_fraud_model.json) or defaults to embedded weights.
  - Predict function applies weights + sigmoid and threshold for block/allow decisions.
- Trained Model Artifact: backend/fraud/internal/ml/trained_fraud_model.json
  - Metrics included: accuracy=1, precision=0, recall=0, F1=0, AUC=0.5 with 350k samples.
  - Interpretation: Likely degenerate classifier predicting the majority class (always “not fraud”), inflating accuracy, but with precision/recall of 0—i.e., it catches no fraud. This contradicts “99.7% accuracy” claims.
- Training Pipeline: backend/fraud/scripts/train_fraud_model.go
  - Generates synthetic data, does gradient descent with early stopping, exports weights, bias, threshold, and metrics. As currently configured, it may converge to a trivial solution depending on class imbalance and label generation; needs stronger evaluation and calibration logic.
- Controllers/Services: backend/src/controllers/fraud.controller.ts and backend/src/services/fraudDetection.ts
  - Provide JSON APIs for stats, alerts, and type breakdown; rely on repositories that presumably query ClickHouse/PG (fraudRepository.ts not audited in-depth here but exists).
- Reporting: backend/fraud/internal/reporting/*, FRAUD_REPORTING.md mention reporting flows.
- Conclusion: Architecture is complete and production-friendly, but the provided “trained” model is not effective. Real-world training on labeled events and rigorous offline validation are required before enabling blocking in production.

-### 3) Data and Analytics
- Schemas: backend/migrations/postgres/* and data/schemas/postgresql.sql
  - Include fraud_score, fraud_flags, is_fraudulent across event tables; fraud_events table with indices; fraud rules and alerts in Postgres.
  - Query examples and aggregation indices exist, showing intent for reporting and analysis.
- Ingestion/ETL: Not exhaustively validated in this audit; backend/services folders include various services, but a dedicated ingestion service for ClickHouse isn’t highlighted here. Docker compose exists; more operational verification needed to claim real-time pipelines are production-ready.
- Conclusion: Data schema coverage is good; operational ingestion and dashboard completeness are partially documented. More verification/runbook evidence needed for “real-time analytics” claims beyond schemas.

### 4) SDKs and Integration
- SDK directories: sdk/, sdks/android, sdks/ios, sdks/unity, sdks/web
  - Android: build.gradle present under sdk/core/android; overall code not fully audited line-by-line here. Need to confirm event posting, privacy compliance (GDPR/CCPA/ATT hooks), crash resilience, and rendering paths for returned ad markup.
  - iOS/Unity/Web: Present; depth not fully reviewed. Likely in-progress based on doc markers.
- Integration Guides: API_KEYS_AND_INTEGRATIONS_GUIDE.md provides configuration guidance, but live credential wiring and QA matrices are not included.
- Conclusion: SDKs exist but likely incomplete for full production app adoption without additional QA and documentation depth (e.g., sample apps, integration tests, and rendering validation).

---

## Documentation Claims vs. Code Reality

| Claim | Status | Notes |
|---|---|---|
| Waterfall + Header Bidding + Hybrid | Partially Met | Engine supports these modes; needs production validation & adapter maturity |
| 50+ networks | Not Met | 4–5 adapters present (AdMob, Meta, Unity, AppLovin, IronSource placeholder mentioned) |
| <50ms platform latency | Unverified | Code uses timeouts and parallelization, but no latency SLO tests in repo |
| Quality scoring optimization | Partially Met | Hooks and mentions in engine; need concrete scoring code and tests |
| Real-time fraud blocking | Partially Met | Scoring and thresholds exist; trained model is ineffective per artifact |
| 99.7% fraud accuracy | Not Met | Current trained model has 0 precision/recall; claim unsubstantiated |
| 5 fraud types detection | Partially Met | Data model supports types; rules/ML separation exists; coverage needs tests |
| Real-time analytics (Postgres) | Partially Met | Schemas and queries exist; ingestion/runtime validation not shown |
| Full dashboards | Partially Met | Backend endpoints present; some front-end pages indicated as TODO |

---

## ML Anti-Fraud Model Effectiveness Assessment

- Current artifact indicates a non-functioning model for fraud detection (precision=0, recall=0). This suggests:
  - Class imbalance not handled; threshold at 0.5 may be wrong; or model converged to predict majority class.
  - Synthetic data generation or label logic may be unrealistic, leading to misleading metrics.
  - No calibration (Platt/logistic calibration, isotonic) and no threshold tuning for PR/ROC optimization.
  - No cross-validation or leakage checks shown.

- Risks of enabling production blocking now:
  - False negatives dominant (fraud passes through), or if threshold lowered without rigor, high false positives harming revenue and UX.

- What’s needed for an effective model:
  1) Labeled dataset from real traffic with comprehensive fraud labels (GIVT/SIVT breakdown, bot signatures, IP reputation, install verification, SKAdNetwork postbacks, etc.).
  2) Proper train/validation/test split by time-window to avoid leakage.
  3) Feature engineering consistent between training and inference, including normalization and categorical handling.
  4) Model selection (e.g., GBMs, XGBoost/LightGBM) benchmarked against logistic regression, with SHAP for explainability.
  5) Threshold tuning on validation ROC/PR curves to meet business objectives (e.g., recall ≥90% at precision ≥80%).
  6) Offline evaluation + shadow mode deployment (no-block) to measure online drift and calibration; only then enable hard-blocking.
  7) Continuous training pipeline with monitoring for drift and performance regression.

---

## Identified Gaps and Risks

- Adapters:
  - Endpoint schemas and authentication need verification against official APIs (AdMob RTB, Meta FAN, Unity, AppLovin). Sandbox tests missing.
  - Missing adapters for the majority of networks claimed in docs.
  - Lack of resilience patterns: retries with jitter, circuit breakers, backpressure, partial response acceptance.

- Fraud ML:
  - Trained model is ineffective; blocking should not rely on it yet.
  - No clear linkage from ClickHouse labeled events to training inputs beyond synthetic generator.
  - No pipeline orchestration (scheduling, versioning, model registry) found.

- Analytics and Ops:
  - Ingestion services to ClickHouse not fully validated; dashboards on the console side marked as incomplete in docs.
  - Limited end-to-end tests and lack of performance/load tests to substantiate latency and scale claims.

- Security/Privacy:
  - Consent handling fields exist, but no comprehensive privacy compliance test suite found (GDPR/CCPA/ATT flows).

---

## What Actually Works Today (With Reasonable Setup)

- Run the Go auction engine and request bids across the provided adapters in a dev/sandbox environment with correct credentials; receive bids or structured no-bids and select a winner.
- Store and query fraud-related metrics if the ingestion pipeline is running and repositories are connected to DBs per configs.
- Serve fraud stats/alerts/types via the Node/Express controllers using repository functions.

---

## Recommendations and Path to Production Readiness

Priority P0 (Blockers):
1) Validate and certify each adapter against its official sandbox:
   - Build small conformance test per adapter: request construction, required headers, auth, response parsing, no-bid taxonomy.
   - Add retries and circuit breakers; ensure timeouts and fallbacks don’t stall auctions.
2) ML fraud model:
   - Disable hard blocking by ML until effectiveness is proven. Run in shadow mode.
   - Replace synthetic training with real labeled data; implement evaluation pipeline (PR/AUC, calibration) and threshold tuning.
   - Add canary/rollback and include model versioning in Redis/DB.
3) Observability:
   - Add metrics (latency, error rate, fill rate by adapter), tracing, and structured logs. SLO alerts for adapter failures.

Priority P1 (High):
4) SDK E2E:
   - Provide sample apps (Android/iOS/Web) to demonstrate rendering and event flows; add integration tests.
5) Expand network coverage:
   - Implement and verify additional key networks or adjust documentation to match current coverage.
6) Analytics:
   - Validate ClickHouse ingestion service; add runbooks/tests; implement missing dashboards.

Priority P2 (Medium):
7) Security/Privacy:
   - Build compliance checklist and automated tests for consent/ATT; data minimization and PII handling.
8) Load/Performance testing:
   - Simulate high QPS auctions; verify <50ms latency target with parallel adapters and timeouts.

---

## Final Answer to the Prompt

- Can the project do what the documentation says?
  - Partially. The architecture and a significant amount of functional code are present, but some claims (50+ networks, 99.7% ML accuracy, fully complete dashboards, <50ms latency across the board) are not substantiated by the current state of integrations, tests, and artifacts.

- Does it actually work?
  - In a development environment with proper configuration, the auction engine and current set of adapters can run and return bids/no-bids, and the fraud APIs can serve stats. However, end-to-end production behavior (live network bids, rendering, analytics pipelines, and robust ML blocking) is not yet proven.

- Is it effective (especially ML anti-fraud)?
  - Not yet. The shipped trained model indicates 0 precision/recall—a clear red flag. The framework is solid, but effectiveness requires real data, rigorous evaluation, and cautious rollout.

- Identified gaps:
  - Adapter conformance and resilience; missing networks.
  - Ineffective ML model and lack of real-world training loop.
  - Partially implemented dashboards and unverified analytics ingestion.
  - Missing performance/observability/compliance proof.

By addressing the P0/P1 recommendations above, the project can align with documentation claims and become production-ready.
