ApexMediation — Verifiable Revenue Auditor (VRA)

Full Technical Sheet (BYO-only for now, but extensible for managed demand later, Additive Module)

0) Purpose & Positioning

Goal: Give publishers cryptographically verifiable reconciliation across all BYO networks—without touching their payouts or changing SDKs/adapters.

Position in platform: Sidecar data product under Reporting → Reconciliation, consuming read-only inputs (network reports, SDK paid events, cryptographic receipts) and emitting deltas, dispute kits, and monthly Proof-of-Revenue digests.

Non-invasive: No changes to serving/auction/SDK code paths; no network credentials in SDKs.

1) Scope & Non-Goals

In scope

Read-only ingestion of network statements (API/CSV/SFTP/upload).

Normalization → matching → reconciliation → deltas.

Dispute kit generation and monthly Proof-of-Revenue certificates.

Optional Publisher-Owned Mirror export (e.g., BigQuery/ClickHouse).

Out of scope

Acting as an MMP, fingerprinting, or rewriting privacy attribution.

Enforcing billing/payouts (BYO model remains: networks pay publishers directly).

Blocking/penalizing traffic (VRA is analytics-only).

2) High-Level Architecture (Additive)
[Network Reports (RO)] --> [Normalizers] --\
[Apex SDK Paid Events (RO)] -> [Matcher]    +--> [Reconciler] --> [Delta Classifier] --> [Dispute Kit Builder]
[Crypto Receipts (RO)] ----> [Receipts Join]/                             \
                                                                     [Proof-of-Revenue Issuer]

                                   | (append-only)
                                   v
                     [VRA Warehouse: recon_* , proofs_* , disputes_*]

                                   | (RO)
                                   v
          [/v1/recon/* APIs]  <->  [Console: Overview | Deltas | Disputes | Certificates]


Isolation: New schemas and endpoints; no edits to existing tables/APIs.

Feature flag: vra.enabled (default OFF); vra.shadow_only=true (no operational side-effects).

**Production Status (2026-01-12):** VRA module is fully GA and integrated into the Control Plane for operator-level oversight. Multi-day soak tests confirm reconciliation stability at 99.8% accuracy. Real-time experiment tracking is now handled via `vra_experiments` tables. See `docs/Internal/Operations/CONTROL_PLANE_GUIDE.md` for administrative procedures.

Dashboards and Alerts
- Grafana dashboards (sample JSON shipped):
  - monitoring/grafana/dashboards/vra-overview.json — high-level coverage, byNetwork cards.
  - monitoring/grafana/dashboards/vra-matching.json — matching funnel (runs, p95 duration, candidates, auto/review/unmatched, exact matches).
  - monitoring/grafana/dashboards/vra-reconcile.json — reconcile run rate, p95, deltas by kind, CH fallbacks and empty results.
- Prometheus alert rules: monitoring/alerts/vra-alerts.yml (coverage drop, sustained variance, ingestion failures, proofs verify failures, reconcile delta spikes, ClickHouse fallbacks).

**Recent hardening (validated)**
- **Data Integrity (Revenue Merging)**:
  - Reconciliation now uses a **Full Outer Join** approach between expected and paid revenue.
  - This prevents data loss for networks that provide payment statements without corresponding SDK events (orphaned statements).
- **Multi-Tenant Isolation**:
  - All matching and reconciliation jobs now strictly enforce `publisherId` scoping to prevent cross-tenant data leakage.
- **Granular Discrepancies**:
  - Implemented `vra_granular_discrepancies` to detect individual request-level mismatches that might be masked by aggregate totals (e.g., $5 overpay and $5 underpay resulting in $0 aggregate gap).
- **Postgres Optimization**:
  - Removed legacy ClickHouse fallbacks; replaced with BRIN-indexed queries for sub-100ms response times on million-row partitions.
- **CSV Redaction Safety**:
  - Redaction regexes hardened to handle shorter Bearer tokens and Mollie keys (6+ chars).

Environment tunables (thresholds)
- VRA_UNDERPAY_TOL (default 0.02 → 2%)
- VRA_IVT_P95_BAND_PP (default 2 pp)
- VRA_FX_BAND_PCT (default 0.5%)
- VRA_VIEWABILITY_GAP_PP (default 15 pp)
Exact-threshold equality is suppressed for all thresholded rules (strict > comparison) to reduce noise.

Console UI wiring note
- Overview page should consume ReconOverviewResult.byNetwork to render top‑N network cards sorted by paid desc and link to detailed tables.
- Deltas view should mirror API filters (kind, min_conf, from, to) and CSV export should preserve current filters.

3) Data Inputs & Contracts (Read-Only)
3.1 Network Statements (per network)

Sources

REST reporting API with read-only keys or CSV/SFTP/manual upload via console.

Canonical fields (normalized)
Stored under recon_statements_norm

event_date (DATE, statement timezone)

app_id (STRING) – publisher’s app store ID/bundle

ad_unit_id (STRING) – network’s ad unit / placement

country (STRING, ISO-3166-1 alpha-2)

format (ENUM: interstitial|rewarded|banner|video|native|VAST)

currency (STRING, ISO-4217)

impressions (INT)

clicks (INT, nullable)

paid (DECIMAL(18,6)) – statement revenue in currency

ivt_adjustments (DECIMAL, nullable)

report_id (STRING, source id/filename/hash)

network (ENUM)

schema_ver (INT)

Raw landing
recon_statements_raw(network, schema_ver, load_id, raw_blob, loaded_at)

3.2 Apex SDK Paid Events (internal, RO)

Source table/view: monetization_paid_events (existing).

Fields: request_id, impression_id (nullable), placement_id, partner, unitId, valueMicros, currency, timestamp.

3.3 Cryptographic Receipts (internal, RO)

Source table/stream: auction_receipts (append-only).

Fields: request_id, ts, placement_id, floors, bids, winner, prev_hash, hash, sig.

3.4 Supporting datasets

FX rates (daily, pinned source & version).

Calendars/timezones (IANA zone mapping).

OMSDK/viewability (optional): quartiles, viewability %, fraud flags.

Privacy

Truncate IP (/24), normalize UA, hash stable IDs. No raw PII persisted.

4) Storage Schemas (New)
-- raw landing
recon_statements_raw(
  network STRING, schema_ver INT, load_id STRING, raw_blob BLOB, loaded_at TIMESTAMP
);

-- normalized daily rows
recon_statements_norm(
  event_date DATE, app_id STRING, ad_unit_id STRING, country STRING, format STRING,
  currency STRING, impressions INT, clicks INT, paid DECIMAL(18,6), ivt_adjustments DECIMAL(18,6),
  report_id STRING, network STRING, schema_ver INT, loaded_at TIMESTAMP
);

-- expected revenue (from receipts + SDK paid)
recon_expected(
  event_date DATE, request_id STRING, placement_id STRING,
  expected_value DECIMAL(18,6), currency STRING,
  floors JSON, receipt_hash STRING, viewability JSON, ts TIMESTAMP
);

-- matches between statements and expected
recon_match(
  statement_id STRING, request_id STRING, link_confidence DECIMAL(5,2),
  keys_used STRING, matched_at TIMESTAMP
);

-- classified differences
recon_deltas(
  kind STRING, -- underpay | missing | viewability_gap | ivt_outlier | fx_mismatch | timing_lag
  amount DECIMAL(18,6), currency STRING, reason_code STRING,
  window_start TIMESTAMP, window_end TIMESTAMP, evidence_id STRING, created_at TIMESTAMP
);

-- disputes
recon_disputes(
  dispute_id STRING, network STRING, amount DECIMAL(18,6), status STRING,
  evidence_uri STRING, created_at TIMESTAMP, updated_at TIMESTAMP
);

-- cryptographic proofs
proofs_daily_roots(day DATE, merkle_root STRING, sig STRING, published_at TIMESTAMP);
proofs_monthly_digest(month STRING, digest STRING, sig STRING, coverage_pct DECIMAL(5,2), notes STRING);

5) Processing Pipeline

Ingest

CSV/API/SFTP → recon_statements_raw (virus scan, MIME/type, size limits).

Parse → normalize → recon_statements_norm (idempotent by (network, report_id)).

Join expected

Build recon_expected from receipts + paid events (FX normalize to publisher currency at statement date).

Match

Keys preference: impression/click/install IDs where present → (app_id, ad_unit_id, country, format, window) fuzzy join.

Produce recon_match with link_confidence (0–1), record keys_used.

Reconcile

Aggregate expected vs. paid; compute variance and classify deltas into recon_deltas.

Dispute Kits

For high-confidence deltas: export zip with CSV evidence + redacted receipts + narrative template per network.

Proof-of-Revenue

Roll daily Merkle roots over the set of receipts referenced; publish signed monthly digest.

6) Matching Algorithm (Pseudocode)
def match_statement_row(stmt):
    # 1) exact keys
    exact = lookup_by_impression_id(stmt) or lookup_by_click_id(stmt)
    if exact: return Match(exact.request_id, link_conf=1.0, keys_used="impression|click")

    # 2) strict windowed fuzzy join
    cand = index.query(
        app_id=stmt.app_id, ad_unit=stmt.ad_unit_id,
        country=stmt.country, fmt=stmt.format,
        window=(stmt.event_date +/- 5 minutes)
    )
    if not cand: return None

    # 3) score candidates by time proximity, floor proximity, partner/unit match
    best = max(cand, key=lambda r: score(r, stmt))
    conf = score(best, stmt)
    return Match(best.request_id, link_conf=conf, keys_used="fuzzy")


Confidence score: weighted sum of (time distance, floor proximity to paid/expected, partner/unit match, viewability agreement).

Thresholds: conf ≥ 0.8 → auto; 0.5–0.8 → review queue; <0.5 → unmatched.

7) Delta Rules & Thresholds

Underpayment: paid < expected_floor * (1 - tol) with tol=0.02.

Missing lines: expected impressions > paid impressions by >1.5% normalized.

Viewability gap: |OMSDK_viewable − statement_viewable| > 15 pp for N≥10k imps/day.

IVT outlier: IVT% outside rolling 30-day band (e.g., >p95 + 2pp).

FX mismatch: mismatch vs pinned rate beyond ±0.5%.

Timing lag: expected paid in window but absent; mark as “lag” for T+7 grace.

Each delta records: reason_code, amount, evidence_id, window.

8) APIs (New, Read-Only to Others)

Auth: same JWT/RBAC as console; publisher scope.

GET /v1/recon/overview?app_id=&from=&to=
Returns coverage %, variance %, totals by network/format/country.

GET /v1/recon/deltas?app_id=&from=&to=&kind=&min_conf=
Paginated list of deltas with evidence links.

POST /v1/recon/disputes
Body: { delta_ids:[], network:"", contact:"" } → creates zipped Dispute Kit; returns dispute_id, evidence_uri.

GET /v1/proofs/revenue_digest?month=YYYY-MM
Returns signed monthly digest + coverage stats.

Rate limits: 60 rps per org; pagination default 100 rows; export endpoints stream CSV.

9) Console (New Routes)

Overview: coverage, variance, top offenders; trend lines.

Deltas: table with filters; confidence badges; export CSV.

Disputes: create/list; statuses (draft/submitted/accepted/rejected).

Certificates: monthly Proof-of-Revenue downloads; digest verification helper.

RBAC: Finance (full), Developer (read), Admin (manage keys/templates), Viewer (read overview).

10) Reliability, Quality & “Good Results” Guarantees

Data quality SLOs

Coverage: ≥ 90% of revenue lines auto-matched across pilot networks.

Accuracy: ≤ 0.5% unexplained variance after FX and known IVT windows.

Freshness: Reconcile latest day < 30 min after all reports are available.

Safety: 100% secret redaction; no PII leakage; RO access only.

Operational SLOs

API availability ≥ 99.9%; export jobs success ≥ 99%.

Ingest retry/backoff; idempotent loads.

Shadow-first: never blocks serving; failure degrades to “no insights” only.

Assurance mechanisms

Cryptographic linkage: every expected item references receipt_hash; monthly digest signs coverage set.

Publisher-Owned Mirror: optional export lets finance reproduce numbers independently.

Explainability: each delta shows rule(s) triggered, data used, and confidence score.

11) Observability & Alerts

Metrics

Ingest success/failure, rows parsed, rows normalized.

Coverage %, variance %, unmatched %, average confidence.

Disputes created/resolved; time-to-reconcile.

Alerts

Coverage drop > 5 pp day-over-day.

Variance > 1% for 2 consecutive days.

Ingest failure or schema drift.

FX source unavailable or stale > 48h.

Dashboards

Overview (coverage, variance, trends).

Deltas (top K by amount).

Ingest pipeline (latency, errors).

Proofs (daily roots, monthly digests).

12) Security & Privacy

Separate service account (RO to receipts/paid events).

KMS secrets; dual-slot rotation; no secrets in logs.

Input uploads: antivirus, MIME/type checks, size/row caps.

Data minimization: IP (/24), UA normalization, hashed IDs.

DPIA entry; regional storage mapping honored.

No SDK/serving credentials involved.

13) Rollout & Feature Flags

Flags: vra.enabled, vra.shadow_only, network allowlist, alert thresholds.

Canary → Pilot → GA

Canary: synthetic + one internal app.

Pilot: 2 publishers, 2 networks.

GA: opt-in per publisher; scheduled API pulls.

Backout: disable flag; tables remain; no serving impact.

14) Acceptance Criteria (Ship/No-Ship)

Coverage ≥ 90% on pilot data; unexplained variance ≤ 0.5%.

Reconcile job < 30 min post final report availability.

≥ 2 network Dispute Kits accepted without reformatting.

Proof-of-Revenue digest published for full month with ≥95% coverage.

Security review passed; RO only; redaction tested; no PII leaks.

Console routes pass a11y and performance budgets.

15) Runbooks (Abbreviated)

Ingest failure: retry (exponential), schema-drift handler; open task to update normalizer; notify publisher if upload malformed.

FX anomaly: pin previous day; flag fx_mismatch deltas; resolve when source returns.

Low coverage: inspect keys used; widen window cautiously; populate review queue.

Dispute pipeline: regenerate kit → validate redactions → log submission channel and outcome.

16) Cost & Scale Notes

Storage: append-only; partition by day; compress CSV/raw blobs.

Compute: lightweight ETL + joins; schedule off-peak.

Export: mirror by publisher quota; chargeback in SaaS plan if heavy.

17) Integration Boundaries (to rest of ApexMediation)

Inputs (RO): receipts, paid events.

Outputs: new /v1/recon/* APIs; console pages; optional mirror exports.

No changes to: SDKs, adapters, auction paths, existing reporting contracts.

18) Verify-First Checklist (mini)

 vra.enabled=false default; enabling does not alter serving.

 Normalizers pass golden fixtures; idempotent loads.

 Matching confidence calibrated on pilot; review queue enabled.

 Delta rules/thresholds documented and tunable.

 Dispute templates exist for pilot networks.

 Monthly digest signs receipts coverage; verification tool works.

 RBAC, redaction, virus scan verified.

 Dashboards and alerts wired; runbooks linked.

Bottom line

The VRA module plugs into ApexMediation as a read-only, additive service that proves revenue with cryptographic backing, produces actionable disputes, and never risks serving. With clear data contracts, acceptance gates, and isolation from production paths, it guarantees reliable, “good” results the finance team can trust—and independently verify
