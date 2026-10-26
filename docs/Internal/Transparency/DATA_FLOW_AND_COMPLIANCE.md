### ApexMediation Data Flow, Transparency, and Compliance (2025-12-06)

Purpose
- Document the end-to-end data flows for ad requests/responses, auction telemetry, transparency artifacts, usage/billing, and reconciliation.
- Define how consent and lawful basis shape collection/processing.
- Specify hashing/redaction rules and export interfaces for operators and publishers.

Scope
- SDKs: iOS/tvOS, Android/TV, Unity, Web (BYO-first)
- Backend/API: request intake, telemetry, transparency exports, usage aggregation, VRA pipeline
- Console: observability/explorer views, downloads
- Third parties: Ad networks (BYO adapters), Mollie, Resend

---

### 1) Client → Backend request and telemetry

1.1 Request envelope (SDKs)
- Minimal identifiers only; no raw device IDs in absence of consent. Key fields:
  - `request_id` (UUIDv4)
  - `timestamp`
  - `app_id`, `placement_id`, `platform`, `sdk_version`
  - Consent snapshot (normalized): `gdpr_applies`, `tc_string?`, `us_privacy?`, `coppa?`, `limit_ad_tracking?`
  - Context: `country`, `connection`, `os_version`, `device_class` (coarse)
  - In BYO pipeline: `enabled_adapters` (names only), `whitelist?` (debug/test only)

1.2 Transparency artifacts (SDKs)
- For each request, SDKs attach hashed transparency fields:
  - `auction_root` (hash chain root per day)
  - `bid_commitments[]` (per adapter/network): `sha256(nonce || network_name || price || placement_id || day_key)`
  - `no_bid_reasons[]` with redacted strings/enums
  - `win_reason`/`lose_reason` (enums) and `clearing_price` (decimal)
- PII never included in transparency artifacts. Device IDs (IDFA/GAID) are never transmitted without lawful basis (ATT allow/TCF consent). When present, they are hashed+salted for storage and never surfaced in exports.

1.3 Backend intake
- API validates envelope, normalizes consent flags, and persists:
  - `requests` fact table (app/placement/context/consent/timestamps)
  - `auction_artifacts` (bid commitments, reasons, prices) with `request_id` FK
  - `adapter_spans` (start/finish/error taxonomy) — tags `strategy`, `sdk_mode`, `platform`
- PII redaction and hashing enforced at write time. See Section 4.

---

### 2) Usage, Billing, and Reconciliation

2.1 Usage aggregation
- Daily job computes app/network usage from `requests`/`auction_artifacts` and network callback logs.
- Results stored in `usage_daily` and `usage_app_tiers` (Starter/Growth/Scale/Enterprise) and feed Mollie events in staging.

2.2 Billing/Mollie
- Mollie events derive from `usage_daily` and tier policies. Webhooks update invoice states (open→paid) and are recorded in `billing_events` (no secrets stored).
- PDFs are saved with operator payment text: Bel Consulting OÜ + Wise SEPA/ACH.

2.3 VRA (Validation/Reconciliation/Audit)
- Ingest network statements (CSV/XLSX/PDF) into `recon_statements_norm`.
- Expected revenue from `usage_daily` populates `recon_expected`.
- VRA pipeline classifies into `recon_match`/`recon_deltas` (under/FX mismatch, etc.).
- Proof‑of‑Revenue: daily Merkle roots + monthly digest stored; verification tool recomputes hashes to assert immutability.

---

### 3) Operator and Publisher exports

3.1 Console transparency views
- Timelines with adapter spans; filters by app/placement/date/network.
- No‑bid reasons; win/lose reasons; clearing prices; pagination.
- Download: per‑request CSV/JSON exports filtered by time/app/network with only redacted fields.

3.2 API exports (documented separately)
- `/api/v1/transparency/exports` — server‑side pagination + filterable by `app_id`, `placement_id`, `date`, `network`. Artifacts include:
  - `request_id`, `timestamp`, `adapter`, `clearing_price`, `no_bid_reason?`, `win_reason?`, `auction_root` hash, and `bid_commitment` hash
- `/api/v1/reconciliation/exports` — statements, expected vs actual, delta categories.

3.3 Retention & access
- Operator exports retain 6–12 months; publisher exports 90 days by default (configurable). Download actions logged.

---

### 4) Privacy and lawful basis (GDPR/CCPA/COPPA)

4.1 Consent mapping
- iOS: ATT gating for IDFA strictly enforced. No IDFA access when denied. SKAdNetwork flows are used for attribution without IDFA.
- Android: UMP Consent SDK; use App Set ID in place of GAID when required; respect Privacy Sandbox flags as applicable.
- Global: TCF v2, US Privacy string, COPPA flags normalized in consent snapshot.

4.2 Redaction and hashing
- Device identifiers (if present with lawful basis) are hashed+salted at write and never returned in exports.
- Free‑text fields are sanitized; error stacks trimmed; secrets never logged.
- Telemetry includes only structured fields with allowlisted keys.

4.3 Data minimization & retention
- Only minimal fields needed for transparency and billing are persisted.
- Retention windows: requests/auction artifacts 12 months; adapter spans 90 days (configurable); billing/VRA per statutory requirements.

---

### 5) Business logic checks and operator responsibilities

- Ensure apps/placements have coherent waterfall priorities in BYO mode; enable test networks only for sandbox.
- Verify that consent snapshots are present for all requests in regulated geos; refusals imply no personalized IDs.
- Confirm Mollie usage tiers and Starter cap transitions reflect `usage_daily` aggregations.
- Confirm VRA delta categories align with contracts; investigate under‑reporting.
- Regularly export transparency artifacts and VRA deltas for archives.

---

### 6) Evidence capture (staging)
- Store CSV/JSON/PDF exports and screenshots under `evidence-YYYY-MM-DD/` according to the networked runbook.
- Attach logs showing `/ready` only depends on Postgres/Redis and security headers present on Console/Website.
