# VRA Runbook

Operational guide for the Variance & Reconciliation Analytics (VRA) system. All features are additive and gated by feature flags: `VRA_ENABLED` and `VRA_SHADOW_ONLY`.

Use the quick links below to jump to a section.

- [VRA Runbook](#vra-runbook)
  - [Operator Quick Start](#operator-quick-start)
  - [Ingestion](#ingestion)
  - [Matching](#matching)
  - [Reconcile](#reconcile)
  - [Deltas](#deltas)
  - [Coverage](#coverage)
  - [Proofs](#proofs)
  - [Read-model troubleshooting](#read-model-troubleshooting)
  - [Backfill and CLIs](#backfill-and-clis)
  - [Canary and Pilot](#canary-and-pilot)
  - [Rollback](#rollback)
  - [Dashboards \& Alerts — Link Verification](#dashboards--alerts--link-verification)

---

## Operator Quick Start

Enable canary (read-only):
```
export VRA_ENABLED=true
export VRA_SHADOW_ONLY=true
```

Quick validation:
- Call `GET /api/v1/recon/overview` → 200, conservative payload
- Call `GET /api/v1/recon/deltas.csv` → header + 8 columns, sanitized `reason_code`
- Hit `/metrics` after Overview to see `vra_coverage_percent` and `vra_variance_percent`
- Open Grafana dashboards; ensure panels render and link to Runbook anchors

Rollback:
```
export VRA_ENABLED=false
```

Notes:
- All surfaces are read-only; disputes remain shadow-only by default.
- For CI guidance see `docs/Internal/VRA/CI_SMOKE.md`.

Planning note (2025-11-25):
- Live canary smoke validation is ready to run with `backend/scripts/vraCanarySmoke.sh`. It remains pending an operator-provided environment: either a staging `API_URL` (and optional `AUTH_TOKEN`) or a local `DATABASE_URL` to boot the API. Once provided, run the script and attach the output under the Evidence appendix.

Defaults when not provided
- API_URL: `http://localhost:3000`
- Auth: no `Authorization` header
- Time window: last 24 hours (`FROM=now-24h`, `TO=now`)
- Flags: `VRA_ENABLED=true`, `VRA_SHADOW_ONLY=true`
- Backfill rehearsal: safe dry-run on ≤3‑day window; exit code 10 (WARNINGS) is treated as non‑fatal

Canary smoke (script)
- Use the provided read‑only smoke script to validate canary in <2 minutes:
```
# Example (local defaults)
bash backend/scripts/vraCanarySmoke.sh

# With explicit API URL and time window
API_URL="https://api.apexmediation.ee" \
FROM="2025-11-01T00:00:00Z" \
TO="2025-11-02T00:00:00Z" \
bash backend/scripts/vraCanarySmoke.sh

# With auth
API_URL="https://api.apexmediation.ee" \
AUTH_TOKEN="<jwt>" \
bash backend/scripts/vraCanarySmoke.sh
```
Pass criteria:
- Overview returns 200; Deltas CSV returns 200 and header line matches; `/metrics` contains `vra_coverage_percent` and `vra_variance_percent` after an Overview call.
Exit codes:
- 0 OK; 1 Overview failed; 2 CSV failed/header mismatch; 3 Gauges missing; 4 Usage error.

---

Note on links and alerts
- All Grafana dashboard links and Prometheus alert `runbook_url` anchors were cross-checked to open the correct sections in this Runbook. See "Dashboards & Alerts — Link Verification" at the end of this document.

---

## Ingestion

Symptoms
- CSV uploads or loaders fail; parse errors increase.
- Panel: “Ingestion reliability” (Overview dashboard) showing spikes in failures by reason.

Signals / Metrics
- `vra_ingest_parse_total{success}` — count of parsed rows (by success=true/false).
- `vra_ingest_load_total{success}` — count of loaded rows (by success=true/false).
- Logs are redacted: emails, Bearer/JWT, Adyen keys, long numerics are masked.

Actions
1) Check recent deployments and feature flags.
2) Inspect ClickHouse for schema drift; confirm CSV headers match allowlist.
3) If drift is confirmed, update mapping or roll back until mapping is fixed.

---

## Matching

Symptoms
- Candidate counts drop unexpectedly; match p95 latency spikes.
- Panel: “Matching funnel” and “Match duration p95 (seconds)” in `vra-matching.json`.

Signals / Metrics
- `vra_match_candidates_total` — candidates enumerated.
- `vra_match_auto_total`, `vra_match_review_total`, `vra_match_unmatched_total` — funnel progression.
- `vra_match_duration_seconds_bucket{outcome}` — duration histogram with outcome label.

Actions
1) Validate ClickHouse availability and input windows.
2) Ensure determinism tie‑breakers are not regressed (tests should be green).
3) If counters drop with no error, confirm filters/time window correctness.

---

## Reconcile

Symptoms
- Sudden spike in deltas by kind or higher latency.
- Panels: “Reconcile run rate”, “Reconcile duration p95”, “Deltas by kind” in `vra-reconcile.json`.

Signals / Metrics
- `vra_reconcile_runs_total{success}` — reconcile runs.
- `vra_reconcile_duration_seconds_bucket{success}` — duration histogram.
- `vra_reconcile_rows_total{kind}` — emitted deltas by kind.

Actions
1) Confirm threshold tunables are set as expected.
2) Validate strict `>` behavior at thresholds (exact equals suppresses emissions).
3) If analytics read-model issues are suspected, see [Read-model troubleshooting](#read-model-troubleshooting).

---

## Deltas

Symptoms
- CSV export malformed; reason text breaking columns; filters not retained.

Signals / Behavior
- CSV export sanitizes `reason_code`: strips commas/newlines/tabs and escapes quotes.
- Amounts fixed to 6 decimals; confidence to 2 decimals.
- Console Deltas page preserves filters in the CSV link and URL.

Actions
1) Repro with current filters; verify columns == 8.
2) If reasons appear unsanitized, confirm backend is at latest version.
3) For suspected PII in reasons, verify logger/CSV redaction tests are green.

---

## Coverage

Definition
- Share of expected vs observed events in the window. May be approximated in early phases.

Signals / Metrics
- Optional: `vra_coverage_percent` (if enabled) or proxy via deltas volume trends.

Actions
1) Investigate ingestion gaps first; then matching filters; finally thresholds in reconcile.

---

## Proofs

Symptoms
- Verification failures; audit complaints.

Signals / Metrics
- `vra_proofs_verify_total{success}` — verification attempts.
- Logs redact cryptographic fields: `digest`, `signature`, `hash`.

Actions
1) Check digest roots and time windows.
2) If repeated failures, ensure source datasets and time bounds match.

---

## Read-model troubleshooting

Symptoms
- Grafana panels show “query failovers”; API responses are empty despite recent activity.

Signals / Metrics
- `vra_query_fail_total` — safeQuery fallback count (alerts if >0 for 15m).
- `vra_empty_results_total` — expected empty results (safe behavior when no rows exist).
- `db_query_duration_seconds{replica="true"}` — replica latency backing the read-model queries.
- `/ready` guardrail panels in Grafana chart replica lag (`pg_stat_replication`), staging backlog (`analytics_ingest_buffer_size` / `pg_stat_user_tables`), and cache hit ratio from `pg_stat_database`.

Actions
1) Check Postgres replica health/lag:
   ```sql
   SELECT application_name,
          EXTRACT(EPOCH FROM COALESCE(replay_lag, '0 seconds')) AS replay_lag_seconds,
          state
     FROM pg_stat_replication;
   ```
   Lag >15s or no rows triggers the readiness guardrail.
2) Inspect API logs for `VRA safeQuery fallback` entries; correlate `op=` labels with the failing panel or endpoint.
3) Ensure analytics partitions contain data for the requested window:
   ```sql
   SELECT date_trunc('day', timestamp) AS day,
          COUNT(*)
     FROM revenue_events
    WHERE timestamp >= NOW() - INTERVAL '7 days'
    GROUP BY 1
    ORDER BY 1 DESC;
   ```
4) If replicas are down or far behind, fail traffic over to healthy instances (or temporarily disable VRA via `VRA_ENABLED=false`).

Verification (partition visibility)
```sql
SELECT child.relname AS partition,
       pg_size_pretty(pg_total_relation_size(child.oid)) AS size,
       child.reltuples AS approx_rows
  FROM pg_inherits
  JOIN pg_class parent ON pg_inherits.inhparent = parent.oid
  JOIN pg_class child ON pg_inherits.inhrelid = child.oid
 WHERE parent.relname IN ('recon_expected','recon_deltas')
 ORDER BY child.relname DESC;
```

---

## Backfill and CLIs

Safety guardrails
- Max window: 3 days; Max limit: 10k rows. Use `--force --yes` to bypass.
- Required date bounds for `--step all`: `--from`, `--to` (ISO timestamps).
- Exit codes: 0 = OK, 10 = WARNINGS (dry‑run no writes), 20 = ERROR/invalid args.

Commands
```bash
# Orchestrator dry‑run
node backend/scripts/vraBackfill.js --from 2025-11-01T00:00:00Z --to 2025-11-03T23:59:59Z --step all --dry-run true

# Expected builder (stage) with caps
node backend/scripts/vraBuildExpected.js --from 2025-11-01T00:00:00Z --to 2025-11-02T00:00:00Z --dry-run true

# Reconcile (stage)
node backend/scripts/vraReconcile.js --from 2025-11-01T00:00:00Z --to 2025-11-02T00:00:00Z --dry-run true

# Match (stage) — thresholds in [0,1]
node backend/scripts/vraMatch.js \
  --from 2025-11-01T00:00:00Z \
  --to   2025-11-02T00:00:00Z \
  --autoThreshold 0.80 \
  --minConf 0.50 \
  --dry-run true

# Issue monthly proofs digest (YYYY-MM)
node backend/scripts/vraIssueProofs.js --month 2025-11 --dry-run true
```

Resume
- Orchestrator supports checkpoints; reruns skip completed stages.

Exit code semantics
- 0 (OK): Work completed with writes enabled (or meaningful dry‑run with non‑zero actions for some tools).
- 10 (WARNINGS): Dry‑run or no work to perform; orchestration treats this as non‑fatal and continues.
- 20 (ERROR): Invalid args, guardrail violation, or initialization failures (legacy ClickHouse clusters may still throw when operators keep them online).

Guardrails
- 3‑day window cap enforced by `vraBuildExpected.js`, `vraMatch.js`, `vraReconcile.js` unless `--force --yes`.
- `--limit` > 10k refused by expected builder unless `--force --yes`.
- Timestamps must be ISO‑like; inverted windows (`from > to`) are rejected.

Sample dry‑run output (orchestrator)
```
[VRA Backfill] Window: 2025-11-01T00:00:00Z → 2025-11-03T23:59:59Z (dry-run)
[VRA Backfill] Stages: ingestion → expected → matching → reconcile → proofs
[VRA Backfill] (ingestion) — operator-driven; ensure statements are ingested via vraIngestCsv.js (dry-run first).
[VRA Backfill] (expected) running vraBuildExpected.js for window
[VRA Backfill] (matching) running vraMatch.js with thresholds
[VRA Backfill] (reconcile) running vraReconcile.js for window
[VRA Backfill] (proofs) running monthly digest issuance for the month of --from
```

Real transcript (example, local dev without ClickHouse)
```
$ node backend/scripts/vraBackfill.js \
    --from 2025-11-01T00:00:00Z \
    --to   2025-11-02T00:00:00Z \
    --step all \
    --dry-run true
[VRA Backfill] Window: 2025-11-01T00:00:00Z → 2025-11-02T00:00:00Z (dry-run)
[VRA Backfill] Stages: ingestion → expected → matching → reconcile → proofs
[VRA Backfill] (ingestion) — operator-driven; ensure statements are ingested via vraIngestCsv.js (dry-run first).
[VRA Backfill] (expected) running vraBuildExpected.js for window
Failed to initialize ClickHouse: connect ECONNREFUSED 127.0.0.1:8123
[VRA Backfill] Stage "expected" failed: vraBuildExpected.js exited with code 20
```

Notes
- Checkpoint writes are best‑effort; if `logs/` is not writable, the orchestrator logs a warning like:
  `Warning: failed to write checkpoint file: logs/vra-backfill-checkpoints.json - EACCES: permission denied` and continues.
- Non‑zero child exit codes propagate to the stage and the orchestrator exits with 10 (WARNINGS) when any stage fails in dry‑run/no‑work scenarios. This is expected in environments without ClickHouse.

Checkpoints
- File path (default): `logs/vra-backfill-checkpoints.json`
- Behavior: completed stages are marked `{ done: true, at: <ISO>, dryRun: <bool> }`, and skipped on rerun.

ClickHouse TTL migrations (retention — legacy)
- The dedicated ClickHouse migrator (`npm --prefix backend run clickhouse:migrate`) was removed after the Postgres cutover. This section is retained for historical context only; modern environments do not run TTL automation, and any remaining ClickHouse clusters should be managed manually if they are still online.

TTL verification (ops)
1) After applying the up migration, run:
```
SELECT name, ttl
FROM system.tables
WHERE database = currentDatabase()
  AND name IN (
    'recon_statements_raw','recon_statements_norm','recon_expected',
    'recon_match','recon_deltas','recon_disputes','proofs_daily_roots'
  );
```
- Expected: non‑empty TTL expressions on all listed tables; `proofs_monthly_digest` intentionally absent.

2) After the down migration, re‑run the query.
- Expected: `ttl` column empty for all listed tables (TTL removed).

---

## Canary and Pilot

Flags
- Canary: `VRA_ENABLED=true`, `VRA_SHADOW_ONLY=true`.

Enable/disable snippets (copy‑paste)
```bash
# Enable canary (read‑only, shadow‑first)
export VRA_ENABLED=true
export VRA_SHADOW_ONLY=true

# Disable (rollback)
export VRA_ENABLED=false
export VRA_SHADOW_ONLY=true   # keep shadow true by default
```

Gates (Pilot)
- Coverage ≥ 90%.
- Unexplained variance ≤ 0.5%.
- Proofs verification OK.

Checks
- `vra_query_fail_total` flat; `vra_empty_results_total` follows expected trends.

Pilot‑gate metrics
- Gauges surfaced from Overview best‑effort: `vra_coverage_percent{scope="pilot"}`, `vra_variance_percent{scope="pilot"}`. Alerts map to these and link back to this Runbook.

Quick validation checklist (after enabling canary)
- [ ] Call `GET /api/v1/recon/overview` and confirm 200 with conservative payload.
- [ ] Call `GET /api/v1/recon/deltas.csv` and confirm header + 8 columns; CSV filename includes `_from_to_` when window provided.
- [ ] Hit `/metrics` at least once after Overview call; verify `vra_coverage_percent` and `vra_variance_percent` are present.
- [ ] No 5xx spikes in API logs; logger output shows redacted fields for suspicious strings.
- [ ] Grafana panels render without errors and Runbook links open the correct anchors.

Grafana panels to pin/watch during canary
- Matching: p95 duration and funnel counters in `monitoring/grafana/dashboards/vra-matching.json`.
- Reconcile & Deltas: p95 duration, deltas by kind, and empty/fallbacks in `monitoring/grafana/dashboards/vra-reconcile.json`.
- Alerts: ensure `vra-alerts` group is loaded; alert annotations link here.

Operator Drill (10 min)
1) Enable flags (canary) as above; deploy/restart if required by your process.
2) Exercise endpoints: Overview, Deltas JSON, Deltas CSV with a narrow window (≤24h).
3) Open Grafana dashboards and confirm timeseries move; check that pilot‑gate gauges exist in `/metrics`.
4) Intentionally request an inverted window to confirm 400 validation paths still work.
5) Verify no read-model hard failures: `vra_query_fail_total` should stay near flat; empties are acceptable early on.
6) If any alert fires, click the runbook link in the alert and follow the steps; record notes.
7) Rollback drill: set `VRA_ENABLED=false` and confirm routes return 404; dashboards remain accessible.

---

## Rollback

Procedure
1) Disable feature flag: set `VRA_ENABLED=false` (or revert rollout change).
2) Validate that API endpoints return empty/safe results and no serving impact occurs.
3) Keep dashboards/alerts enabled to monitor for residual issues.

Notes
- All VRA surfaces are read‑only. Disputes remain shadow‑only unless explicitly enabled.

---

## Dashboards & Alerts — Link Verification

Date: 2025-11-25

Dashboards
- Matching: `monitoring/grafana/dashboards/vra-matching.json` → links to `#matching` anchors in this Runbook. OK.
- Reconcile & Deltas: `monitoring/grafana/dashboards/vra-reconcile.json` → links to `#reconcile` and `#deltas`. OK.

Alerts
- File: `monitoring/alerts/vra-alerts.yml`
  - Coverage drop → `#coverage` anchor. OK.
  - Sustained variance → `#reconcile` anchor (investigation path). OK.
  - Ingest schema drift → `#ingestion`. OK.
  - Proofs verify failures → `#proofs`. OK.
  - ClickHouse fallbacks → `#clickhouse-troubleshooting`. OK.

Notes
- Anchors are stable and case-sensitive; keep titles unchanged to avoid broken links.
- If this Runbook is relocated, update dashboard links and `runbook_url` annotations accordingly.
