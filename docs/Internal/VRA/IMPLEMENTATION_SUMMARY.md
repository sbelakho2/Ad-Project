# Verifiable Revenue Auditor (VRA) — Implementation Summary

Status: Additive rollout (shadow-first). Default OFF. No serving impact.

This document summarizes the implementation that wires the VRA module described in `VRA.md` into the ApexMediation backend, with safe-by-default flags, storage schemas, routes, metrics, ingestion scaffolding, and tests.

Scope delivered in this iteration
- Isolation and Flags
  - New feature flags (environment-driven via `backend/src/utils/featureFlags.ts`):
    - `VRA_ENABLED` (default: false) — guards all VRA routes
    - `VRA_SHADOW_ONLY` (default: true) — dispute creation acknowledges with 202 and does not write side-effects
    - `VRA_ALLOWED_NETWORKS` — optional CSV allowlist for processing scope
- Storage (ClickHouse)
  - New tables created via migrations under `backend/migrations/clickhouse/20251123_181800_vra_tables.*.sql`:
    - `recon_statements_raw`, `recon_statements_norm`
    - `recon_expected`, `recon_match`, `recon_deltas`, `recon_disputes`
    - `proofs_daily_roots`, `proofs_monthly_digest`
  - Historical migrations lived under `backend/scripts/runClickHouseMigrations.js`, but the script was removed after the Postgres read-model cutover; ClickHouse tables remain documented here for archival context only.
- Services
  - `backend/src/services/vra/vraService.ts`:
    - `getOverview(params)` — computes conservative overview using CH `revenue_events` (USD-normalized); expected==paid baseline (variance 0) until recon_expected is populated. Adds per‑network slices in `byNetwork` for UI panels and dashboards.
    - `getDeltas(query)` — reads `recon_deltas` with pagination and confidence filter.
    - `getMonthlyDigest(month)` — fetches proofs from `proofs_monthly_digest`.
  - All ClickHouse interactions are wrapped in `safeQuery` which degrades to empty results to guarantee “no insights” instead of failures.
  - Pilot‑gate metrics (gauges):
    - File: `backend/src/utils/prometheus.ts`
    - Gauges: `vra_coverage_percent{scope}` and `vra_variance_percent{scope}` registered and exported.
    - Publishing: `vraService.getOverview` best‑effort sets `{scope="pilot"}` values; never throws in lightweight envs.
  - Matching Engine (scaffold, library):
    - File: `backend/src/services/vra/matchingEngine.ts`
    - Provides `scoreCandidate(...)` and `matchStatementsToExpected(...)` for Phase A (exact keys placeholder) and Phase B (fuzzy window) matching.
    - Confidence scoring combines time proximity, amount proximity, and unit hints (app/ad unit/country/format) with tunable weights and thresholds.
    - Metrics wired: `vra_match_duration_seconds`, `vra_match_candidates_total`, `vra_match_auto_total`, `vra_match_review_total`, `vra_match_unmatched_total`.
- Ingestion scaffolding (new)
  - Canonical CSV parser + idempotent raw→norm loaders with Prometheus metrics:
    - Files: `backend/src/services/vra/ingestion/statementIngestionService.ts`
    - Metrics: rows parsed, loads, failures
  - Multi-network header mapping → canonical schema for CSV uploads:
    - Files: `backend/src/services/vra/ingestion/networkNormalizers.ts`
    - Supported mappings: AdMob, Unity, AppLovin, ironSource, AdColony, Chartboost, Vungle, Mintegral, Pangle, Meta, Fyber, Smaato, Tapjoy, Moloco, Amazon
  - Operator CLI for ingestion with dry-run and guardrails:
    - File: `backend/scripts/vraIngestCsv.js`
    - Features: `--dry-run`, separator auto-detection (comma/semicolon/tab), strict MIME option, size and row caps, antivirus placeholder hook, structured exit codes (0 OK, 10 WARNINGS, 20 ERROR, 30 SCHEMA_DRIFT reserved, 40 BLOCKED)
  - Unit tests: parser + network mappings
    - Files: `backend/src/services/vra/ingestion/__tests__/statementIngestionService.test.ts`, `backend/src/services/vra/ingestion/__tests__/networkNormalizers.test.ts`

- Expected Builder (Workstream 3) — CLI + Metrics
  - Service: `backend/src/services/vra/expectedBuilder.ts` builds `recon_expected` by joining Postgres transparency receipts with ClickHouse `revenue_events` (idempotent; skips existing IDs; shadow‑safe fallbacks when CH/PG unavailable).
  - Operator CLI: `backend/scripts/vraBuildExpected.js`
    - Usage (dry‑run):
      `node backend/scripts/vraBuildExpected.js --from 2025-11-01T00:00:00Z --to 2025-11-02T00:00:00Z --limit 10000 --dry-run`
    - Usage (write):
      `node backend/scripts/vraBuildExpected.js --from 2025-11-01T00:00:00Z --to 2025-11-02T00:00:00Z --limit 10000`
    - Exit codes: 0 OK, 10 WARNINGS (e.g., wrote 0), 20 ERROR
  - Metrics (Prometheus):
    - `vra_expected_seen_total` — receipts scanned from PG
    - `vra_expected_written_total` — rows written (or would write in dry‑run)
    - `vra_expected_skipped_total` — skipped (existing/no paid match)
    - `vra_expected_build_duration_seconds{outcome}` — duration histogram
- API Routes (guarded)
  - `backend/src/routes/vra.routes.ts` mounted in `routes/index.ts` at `/api/v1` root:
    - `GET /api/v1/recon/overview`
    - `GET /api/v1/recon/deltas`
    - `GET /api/v1/recon/deltas.csv` — streamed CSV export mirroring JSON filters (see below)
    - `POST /api/v1/recon/disputes` — returns 202 in shadow mode
    - `GET /api/v1/proofs/revenue_digest?month=YYYY-MM`
  - All routes require auth and are protected by the read-only rate limiter.
- Controllers
  - `backend/src/controllers/vra.controller.ts` orchestrates requests → services → responses, including input validation and shadow-mode behavior.
  - Deltas query validation (shared by JSON and CSV):
    - `from`, `to`: ISO-like timestamps (optional; defaults applied when absent)
    - `page` ≥ 1; `page_size` ∈ [1..500]
    - `min_conf` ∈ [0..1]
    - `kind` ∈ {`underpay`|`missing`|`viewability_gap`|`ivt_outlier`|`fx_mismatch`|`timing_lag`}
  - Window ordering validation: APIs enforce `from <= to` across JSON, CSV, and Overview endpoints (400 on inverted windows)
  - CSV export escaping/sanitization:
    - Doubles quotes, strips newlines/commas/tabs in `reason_code`; fixes numeric formatting (amount 6 d.p., confidence 2 d.p.) to keep a stable CSV schema
  - CSV filename UX: `Content-Disposition` suggests `recon_deltas_<from>_to_<to>_<timestamp>.csv` when window params are provided
  - Operator tip: When filtering by `kind` and `min_conf`, the CSV mirrors JSON filters exactly. Suggested usage: keep `page_size` reasonably large (<=500) and paginate chronologically by `window_start`.
- Operator CLIs (backfill helpers)
  - `backend/scripts/vraBuildExpected.js` — builds `recon_expected` from PG↔CH join with guardrails (3‑day window cap, `--limit`<=10k unless `--force --yes`). Exit codes: 0 OK, 10 WARNINGS (e.g., no work/dry‑run), 20 ERROR.
  - `backend/scripts/vraMatch.js` — runs the matching engine for a window with thresholds (`--autoThreshold`, `--minConf`); same caps and exit code contract.
  - `backend/scripts/vraReconcile.js` — computes deltas and optionally writes to CH; same caps and exit code contract.
  - `backend/scripts/vraIssueProofs.js` — creates/updates `proofs_monthly_digest` for `--month YYYY-MM`; validates input; exit codes 0/10/20.
  - `backend/scripts/vraBackfill.js` — safe orchestrator scaffold that chains stages (ingestion→expected→matching→reconcile→proofs) with resumable checkpoints and flag forwarding; treats exit code 10 as non‑fatal to continue.
- Tests
  - `backend/src/routes/__tests__/vra.routes.test.ts` verifies:
    - Feature-disabled returns 404
    - Feature-enabled returns 200 with conservative payload when CH is empty
    - Shadow-mode dispute creation returns 202
    - Revenue digest input validation (bad month → 400)
    - CSV export: correct content-type and header line, invalid params → 400, basic rate-limit behavior
    - CSV export: redaction of potential PII/secrets in `reason_code`
  - Ingestion tests verify canonical parser behavior, idempotent loading, and header mappings for major networks.
  - Matching engine unit tests (`backend/src/services/vra/__tests__/matchingEngine.test.ts`) cover scoring properties, auto/review/unmatched paths, and time-window behavior.
  - Redaction unit tests: `backend/src/services/vra/__tests__/redaction.test.ts`
   - Determinism and performance:
     - `backend/src/services/vra/__tests__/matchingEngine.determinism.test.ts` — stable outputs across repeated runs.
     - `backend/src/services/vra/__tests__/matchingEngine.largeN.test.ts` — synthetic large‑N performance sanity with Prometheus counter assertions (gate larger N via `VRA_LARGEN=1`).
   - Controller/CSV redaction:
     - `backend/src/routes/__tests__/vra.csv.redaction.test.ts` — ensures `reason_code` emails/tokens are redacted and commas/newlines are stripped in CSV export.
    - Controllers/services log redaction:
      - `backend/src/routes/__tests__/vra.controllers.rbac.logs.extended.test.ts` — verifies `reason_code` masking on log paths (top-level and nested) via centralized logger.
    - Builder/proofs/dispute-kit log redaction:
      - `backend/src/services/vra/__tests__/expectedBuilder.logsRedaction.test.ts`, `backend/src/services/vra/__tests__/proofs.logsRedaction.test.ts`, `backend/src/services/vra/__tests__/disputeKit.logsRedaction.test.ts` — assert masking of emails/Bearer/JWT, Mollie-like keys, long numerics, and crypto fields (`digest`, `signature`, `hash`).
  - Reconcile boundary tests:
    - `backend/src/services/vra/__tests__/reconcile.boundaries.test.ts` — underpay tolerance exact-equals suppression.
    - `backend/src/services/vra/__tests__/reconcile.fx_viewability.boundaries.test.ts` — FX band and viewability gap thresholds (below/equals/above) with env-driven tolerances.
  - CLI guardrails:
    - `backend/scripts/__tests__/vraBuildExpected.caps.test.ts`
    - `backend/scripts/__tests__/vraMatch.caps.test.ts`
    - `backend/scripts/__tests__/vraReconcile.caps.test.ts`
    - `backend/scripts/__tests__/vraIssueProofs.caps.test.ts`

Observability polish (new)
- Overview enhancements
  - `byNetwork` slice added to `GET /api/v1/recon/overview` payload for quick per‑network breakdown cards and panels.
  - Grafana dashboard updated to slice by `network` (see `monitoring/grafana/dashboards/vra-overview.json`).
- Reconcile & Deltas dashboard
  - Added `monitoring/grafana/dashboards/vra-reconcile.json`:
    - Reconcile run rate and p95 via `vra_reconcile_duration_seconds_*`.
    - Deltas by kind using `vra_reconcile_rows_total{kind=...}`.
    - 24h total deltas and read-model failover / empty result guardrails.
- Metrics/alerts alignment
  - Prometheus metrics used across VRA flows: `vra_query_duration_seconds`, `vra_query_fail_total`, `vra_empty_results_total`, ingestion/expected/matching/reconcile counters and histograms listed in `backend/src/utils/prometheus.ts`.
  - Alert samples aligned to the metric names in `monitoring/alerts/vra-alerts.yml` (coverage drop, sustained variance, ingestion failures, proof verify failures, reconcile delta spikes, analytics failovers), with `runbook_url` annotations pointing to `docs/Internal/VRA/RUNBOOK.md` anchors.
  - Gauges used by alerts: `vra_coverage_percent{scope="pilot"}`, `vra_variance_percent{scope="pilot"}` — set by Overview; alerts reference RUNBOOK anchors.

Infrastructure posture (DigitalOcean, budget‑capped)
- Deployment follows the infra plan in `docs/Internal/Infrastructure/INFRASTRUCTURE_MIGRATION_PLAN.md`:
  - Single DO droplet (2 vCPU / 4 GB) running API, Console, Redis, and Nginx.
  - DO Managed Postgres for primary DB and early analytics.
  - ClickHouse is optional: all VRA queries are wrapped in `safeQuery` and degrade to empty, ensuring read‑only behavior and no serving risk.
  - DO Monitoring for base signals; optional Prometheus/Grafana stack for VRA dashboards with short retention.
 - Matching dashboard
   - Added `monitoring/grafana/dashboards/vra-matching.json` with timeseries p95 panels for `vra_match_duration_seconds_bucket` and funnel counters (candidates/auto/review/unmatched).

RBAC & Logs spot‑checks
- All VRA routes are behind `requireFeature('vraEnabled')`, `authenticate`, and `readOnlyRateLimit`.

---

Operator quick links
- Runbook (Operator Quick Start, canary/pilot, backfill & CLIs): `docs/Internal/VRA/RUNBOOK.md`
- CI smoke (backend-only, DB/observability opt-in): `docs/Internal/VRA/CI_SMOKE.md`
- Canary smoke script (read‑only): `backend/scripts/vraCanarySmoke.sh`
- Dashboards (Grafana JSON): `monitoring/grafana/dashboards/vra-matching.json`, `monitoring/grafana/dashboards/vra-reconcile.json`
- Alerts (Prometheus rules): `monitoring/alerts/vra-alerts.yml` (links to Runbook anchors)
- Disputes respect `VRA_SHADOW_ONLY` — in shadow mode, API returns 202 and performs no writes. Counters: `vra_dispute_shadow_acks_total` (shadow) vs `vra_disputes_created_total` (non‑shadow).
- Logger has centralized redaction for sensitive strings and keys (deep traversal; patterns for JWT‑like strings, OAuth tokens, long hex secrets, long numerics; and masking for structured keys including `reason_code`, `signature`, `sig`, `digest`, `hash`, `prev_hash`).
- VRA CSV and kit builders apply additional redaction for `reason_code`/evidence; controller/service log paths mask `reason_code` even when nested.
- Tests cover redaction and no‑secret logging across ingestion, controllers/services, proofs, and dispute kit.

CLI safety & backfill
- Stage CLIs (`vraBuildExpected.js`, `vraReconcile.js`) enforce safety caps: max window 3 days; `--limit` ≤ 10k unless `--force --yes`. Dry‑run produces exit codes 0/10.
- Orchestrator `vraBackfill.js` requires `--from/--to` for `--step all`, rejects `--force` without `--yes` and non‑positive `--limit`, supports checkpoints/resume and forwards flags (`--dry-run`, `--autoThreshold`, `--minConf`).
- Tests: `backend/scripts/__tests__/vraBuildExpected.*.test.ts`, `backend/scripts/__tests__/vraReconcile.*.test.ts`, `backend/scripts/__tests__/vraBackfill.*.test.ts` cover caps, bypasses, invalid combos, and resume semantics.

CI test discoverability
- Backend Jest config at `backend/jest.config.js` discovers tests under `backend/src/**` and `backend/scripts/**` for both `.ts` and `.js` files; root script `npm run test:backend` executes the suite. A CJS proxy `backend/jest.config.cjs` is present for CI environments that require it.

Docs & Runbooks
- Operator runbook with actionable anchors and CLI recipes: `docs/Internal/VRA/RUNBOOK.md`.
- Cross-links added from dashboards and alerts to runbook anchors for one‑click troubleshooting.

Runbooks and operator tips
- CSV export
  - JSON: `GET /api/v1/recon/deltas`
  - CSV: `GET /api/v1/recon/deltas.csv` — respects the same query params.
  - Filenames: `recon_deltas_<from>_to_<to>_<timestamp>.csv` when window params provided; otherwise `recon_deltas_<timestamp>.csv`.
  - Tip: Keep filenames under 120 chars for better OS portability; avoid using raw app names with spaces in query params if you plan to save directly from the browser.
- Overview usage
  - Use `byNetwork` for top‑N cards on the Console Overview; fall back to `byBreakdown` to render detailed tables.

Rollout readiness
  - Canary settings: `VRA_ENABLED=true`, `VRA_SHADOW_ONLY=true` (no serving impact). Validate metrics on canary: no spikes in `vra_query_fail_total`, expected trends in `vra_empty_results_total` for empty envs.
- Pilot acceptance gates:
  - Coverage ≥90%
  - Unexplained variance ≤0.5%
  - Proofs verification OK (no `vra_proofs_verify_failures_total` increases)
- Finalize runbooks with rollback: set `VRA_ENABLED=false` to disable API surface; jobs/CLIs are safe by design (dry‑run flags, redaction, and RO posture by default).

  Recent additions (matching + reconcile)
  - Matching Engine
    - Exact-key short-circuit: when a statement contains `requestId` that exists in `recon_expected`, it matches with `confidence=1.0` and `keys_used="exact"`.
    - Review queue persistence: mid-confidence matches (0.5–0.8) can be optionally persisted to `recon_match_review` for analyst triage.
    - CLI flags added to `backend/scripts/vraMatch.js`:
      - `--autoThreshold` (default 0.8)
      - `--minConf` (default 0.5)
      - `--persistReview` (boolean)
    - Metrics: `vra_match_exact_total`, `vra_match_review_persisted_total`.
  - Reconcile & Delta Classification (coarse aggregate pass)
    - Rules implemented:
      - `timing_lag`: unmatched expected within window (grace handling to be extended later)
      - `underpay`: residual gap beyond `VRA_UNDERPAY_TOL` (default 0.02)
      - `ivt_outlier`: current IVT% > 30-day p95 + band `VRA_IVT_P95_BAND_PP` (default 2 pp)
      - `fx_mismatch`: current avg `exchange_rate` deviates > band vs 30-day median per currency `VRA_FX_BAND_PCT` (default 0.5%)
      - `viewability_gap`: |OMSDK_viewable − statement_viewable| > gap `VRA_VIEWABILITY_GAP_PP` (default 15 pp)
    - Tunables (env): `VRA_UNDERPAY_TOL`, `VRA_IVT_P95_BAND_PP`, `VRA_FX_BAND_PCT`, `VRA_VIEWABILITY_GAP_PP`
    - Equality handling: exact equality to thresholds is suppressed (strict `>` comparison) to reduce noise.
    - Metrics: `vra_reconcile_duration_seconds`, `vra_reconcile_rows_total{kind}`

Dispute Kit Builder — storage adapters and controller wiring
- Storage adapters (pluggable):
  - `FileSystemDisputeStorage` (dev/CI): writes JSON bundles under a configured directory and returns `file://` URIs.
    - Env: `VRA_DISPUTE_STORAGE=fs`, `VRA_DISPUTE_FS_DIR=/abs/path/for/kits` (default: `logs/vra-kits` under cwd)
  - `S3DisputeStorage` (prod): lazy‑loads `@aws-sdk/client-s3`; writes bundles to S3 and returns presigned URLs (falls back to `s3://` if presign fails).
    - Env: `VRA_DISPUTE_STORAGE=s3`, `VRA_DISPUTE_BUCKET`, `VRA_DISPUTE_PREFIX=vra`, optional `VRA_S3_REGION`, `VRA_S3_ENDPOINT`, and `VRA_DISPUTE_TTL_SEC`.
- Resolver: `resolveDisputeStorageFromEnv()` selects adapter; on misconfig falls back to in‑memory storage.
- Controller integration: `POST /api/v1/recon/disputes` uses env‑resolved storage when `VRA_SHADOW_ONLY=false`. In shadow mode, still returns 202 with `shadow=true` and no writes.
- Bundle hardening: Adds `checksum_sha256` and `ttl_sec` into metadata; CSV evidence is redacted.
- Metrics: `vra_dispute_kits_built_total{network}`, `vra_dispute_kit_failures_total{reason}`.
- Tests: FS adapter write/read (`disputeKitStorage.test.ts`) and kit redaction/persistence (`disputeKitService.test.ts`).

Cryptographic Proofs Issuance (daily roots, monthly digest) — scaffolding
- Service: `backend/src/services/vra/proofsIssuer.ts`
  - Deterministic `computeMerkleRoot(hashes)` and `sha256Hex(...)` helpers.
  - `issueDailyRoot(day, {dryRun})` — computes Merkle root over receipts referenced by `recon_expected` for the day. Uses `receipt_hash` when available else SHA‑256 of `request_id`.
  - `issueMonthlyDigest(month, {dryRun})` — computes digest over monthly daily roots.
  - `verifyMonthlyDigest(month)` — recomputes and checks signature if public key provided.
  - Signing (optional):
    - Env: `PROOFS_SIGNING_PRIVATE_KEY` (PEM) for issuance; `PROOFS_SIGNING_PUBLIC_KEY` (PEM) for verification helper/CLI.
    - If absent, signatures are omitted and verification reports `signature_invalid_or_missing` (non‑fatal for dev).
- CLIs:
  - `backend/scripts/vraIssueProofs.js`
    - Daily: `node backend/scripts/vraIssueProofs.js --daily 2025-11-01 --dry-run`
    - Monthly: `node backend/scripts/vraIssueProofs.js --month 2025-11 --dry-run`
  - `backend/scripts/vraVerifyDigest.js`
    - Verify: `node backend/scripts/vraVerifyDigest.js --month 2025-11`
- Metrics:
  - `vra_proofs_issuance_duration_seconds` (histogram)
  - `vra_proofs_verify_failures_total` (counter)
  - `vra_proofs_coverage_pct{day}` (gauge)
- Tests: `backend/src/services/vra/__tests__/proofsIssuer.test.ts` verifies determinism, daily/monthly dry‑run behavior, and verification failure paths.

  Operator CLIs (ref)
  - Matching batch (with optional review persistence):
    ```
    node backend/scripts/vraMatch.js \
      --from 2025-11-01T00:00:00Z \
      --to   2025-11-02T00:00:00Z \
      --autoThreshold 0.8 \
      --minConf 0.5 \
      --persistReview true
    ```
  - Reconcile (dry-run):
    ```
    node backend/scripts/vraReconcile.js --from 2025-11-01T00:00:00Z --to 2025-11-02T00:00:00Z --dry-run
    ```

Out of scope for this iteration (planned next milestones)
- Full ingestion of network statements and normalization (`recon_statements_*`).
- Building `recon_expected` from receipts + SDK paid events; matching algorithm and confidence calibration.
- Delta classification rules (underpay, missing, IVT, FX, timing) population and review queues.
- Daily Merkle roots rolling up to monthly digests issuance (issuance job + signing integration).
- Console pages for Overview/Deltas/Disputes/Certificates.

Operational notes
- Enabling VRA safely for canaries:
  1. Apply Postgres migrations: `npm --prefix backend run migrate`
  2. Start API with `VRA_ENABLED=true` and keep `VRA_SHADOW_ONLY=true` initially.
  3. Hit `/api/v1/recon/overview` and `/api/v1/recon/deltas` to validate surfacing and rate limits.
  3a. CSV export: `/api/v1/recon/deltas.csv?from=YYYY-MM-DD&to=YYYY-MM-DD&kind=underpay&min_conf=0.8&page=1&page_size=100`
  4. For ingestion trials, use the CLI in dry-run mode first:
     - Example (dry-run):
       `node backend/scripts/vraIngestCsv.js --network unity --schemaVer 1 --loadId unity-2025-11-01 --reportId unity-2025-11-01 --file ./unity.csv --dry-run true`
     - Example (write):
       `node backend/scripts/vraIngestCsv.js --network unity --schemaVer 1 --loadId unity-2025-11-01 --reportId unity-2025-11-01 --file ./unity.csv`
   5. Backfill orchestration (resumable checkpoints):
      - Example:
        `node backend/scripts/vraBackfill.js --from 2025-11-01T00:00:00Z --to 2025-11-03T00:00:00Z --step all --checkpoint logs/vra-backfill-checkpoints.json --dry-run true`
      - Stages: ingestion → expected → matching → reconcile → proofs (wired to stage CLIs; WARNINGS exit code tolerated)

 Monitoring & alerts
 - Grafana dashboard stubs: `monitoring/grafana/dashboards/vra-overview.json`
   - Panels: VRA query durations, matching auto/review/unmatched, reconcile rows by kind, proofs coverage %, ingest rows/loads/failures.
 - Per‑network ingest dashboard: `monitoring/grafana/dashboards/vra-ingest-by-network.json`
   - Panels: rows parsed per network, loads by phase (raw/norm), failures by reason.
 - Alert rules (Prometheus): `monitoring/alerts/vra-alerts.yml`
   - Coverage drop > 5 pp DoD (placeholder metric), unexplained variance > 1% for 48h (placeholder), ingestion failures spike, proofs verify failures.
  5. Dispute Kits storage:
     - Dev (local FS): set `VRA_DISPUTE_STORAGE=fs` (optional `VRA_DISPUTE_FS_DIR`) — kits saved under `logs/vra-kits` by default.
     - Prod (S3): set `VRA_DISPUTE_STORAGE=s3`, `VRA_DISPUTE_BUCKET`, `VRA_DISPUTE_PREFIX`, and region/endpoint as needed.
  6. Proofs issuance (dry‑run recommended first):
     - Daily root: `node backend/scripts/vraIssueProofs.js --daily YYYY-MM-DD --dry-run`
     - Monthly digest: `node backend/scripts/vraIssueProofs.js --month YYYY-MM --dry-run`
     - Verify digest: `node backend/scripts/vraVerifyDigest.js --month YYYY-MM`
- Backout plan: set `VRA_ENABLED=false` (routes return 404). No serving paths affected.

Assurance mechanisms (current)
- Read-only data access; no SDK/auction path modifications.
- Shadow-first disputes; no external submissions.
- Input validation and CH-safe queries ensure uptime if ClickHouse is unavailable (logs a warning; returns empty result).

Acceptance gates coverage
- The module is wired and gated; storage, APIs, and flags are in place with tests.
- Final acceptance metrics (coverage ≥90%, variance ≤0.5%, digests with ≥95% coverage) require subsequent ingestion/matching/delta issuance work.

References
- Product spec: `VRA.md`
- Routes: `backend/src/routes/vra.routes.ts`
- Controllers: `backend/src/controllers/vra.controller.ts`
- Services: `backend/src/services/vra/vraService.ts`
- Migrations: `backend/migrations/clickhouse/20251123_181800_vra_tables.*.sql`
- Flags: `backend/src/utils/featureFlags.ts`