# Migration Studio — Parallel Mediation, Safe Shadowing, and Verified Uplift

Owner: Growth Engineering / Platform
Status: Design + Checklist added (implementation tracked under Section 8 in DEVELOPMENT_TODO_CHECKLIST)
Last updated: 2025-11-11

## What and why
Migration Studio lets a publisher currently using ironSource/AppLovin/MAX/etc. try our stack without risky rip-and-replace.

- Drop in our SDK once. From the dashboard, enable “Mirror N%” for a placement.
- We clone their mediation setup (waterfalls/instances/line items) into our system.
- We run safe, parallel mediation in shadow alongside their incumbent.
- We show side-by-side, cryptographically verifiable lift: eCPM, fill, latency, and net revenue.
- Roll-up answers the core question: “If 100% routed through us in the last 14 days, projected +X% revenue.”
- **Performance Optimized**: Postgres-native snapshots using BRIN indexes and `sample_percentile_if_large()` for sub-millisecond p95 latency on high-volume partitions.
- **Zero-Trust Security**: Server-side authoritative override of experiment `mode` (shadow vs. mirroring) to prevent client-side escalation.

This targets three adoption friction points:
- Removes switching fear: parallel, reversible, scoped (per placement, % mirrored).
- Shortens sales cycle: live migration simulator instead of pitch decks.
- Supports value-based pricing: uplift claims are backed by Ed25519 transparency and ML fraud adjustments.

## Principles
- Additive: no changes to core auction loop; use flags/targeting and existing routing/logging hooks.
- Transparent: Ed25519 canonical logs and verifiable reports for CFO/leadership.
- Safe-by-default: strict guardrails limit traffic, protect latency and revenue; one-click off switch.

## Architecture (high level)
- Control Plane service: “migration-studio” microservice (Node/Go) for import/mapping, experiment assignment, and reporting API.
- Console UI: new section “Migration Studio” with placement selector, mirroring sliders, and comparison reports.
- SDK: uses existing feature flag/targeting hooks to mark impressions as Control vs Test; no SDK rewrite.
- Data: impressions/events labeled with experiment metadata; diffs computed over rolling windows.
- Trust layer: Ed25519 canonical logs reference control/test decisions; report signatures and JSON export.

## Data model (concepts)
- Experiment: scope (org, app, placement), start/stop, mirror_percent, assignment seed, and objectives.
- Mapping: adapter/instance equivalence map to reconstruct incumbent demand stack.
- Assignment: deterministic hash(user/device, placement, seed) < mirror_percent → Test; else Control.
- Metrics: eCPM, fill, latency, IVT-adjusted revenue, error taxonomies.

## Control Plane APIs (sketch)
- POST /api/v1/migration/experiments — create experiment for placement(s)
- POST /api/v1/migration/import — upload CSV/JSON or connect to incumbent API; returns mapping draft
- PUT  /api/v1/migration/mappings/:id — confirm mapping; resolve adapters/instances
- POST /api/v1/migration/activate — start mirroring with percent and guardrails
- GET  /api/v1/migration/reports/:expId — return side-by-side metrics and signed JSON

## Import pipelines
- CSV uploader template for common formats (ironSource/MAX line items).
- Optional API connectors (publisher-provided creds) to fetch live setup.
- Manual mapping UI to resolve adapters (15+ adapters supported by our stack).

### CSV templates & validation
- Templates live in `docs/Features/MigrationStudio/templates/` for both ironSource and AppLovin/MAX exports.
- Each template includes the canonical headers we validate against (`network/provider`, `instance_id`, `InstanceID`, `Label`, `rank`/`waterfall_position`, `ecpm_cents`/`floor_cents`).
- The backend parser normalizes these headers, trims values, and raises descriptive errors for missing required columns or malformed numeric fields.
- Add new provider formats by extending `backend/src/utils/migrationCsvParser.ts` with the appropriate column aliases and validation rules.

### Signed comparisons & verification
- Import runs now produce an Ed25519-signed comparison artifact summarizing eCPM, fill, latency (p50/p95), and IVT-adjusted revenue.
- Backend implementation: `backend/src/services/migrationComparisonSigner.ts` (payload canonicalization + signing) surfaced via controller responses.
- Console surfaces the comparison and exposes a one-click copy action for the signature payload (`console/src/components/migration-studio/ImportWizard.tsx`).
- Verification clients can base64-decode the payload and validate against the published public key (`signature.public_key_base64`).

### Sandbox validation
- Run `SKIP_DB_SETUP=true npx ts-node scripts/migration-import-sandbox.ts` inside `backend/` to execute an end-to-end sandbox import (ironSource connector → suggested adapters → finalize import).
- The script simulates a publisher placement, auto-assignment to Apex adapters, emits the signed comparison payload, and verifies the Ed25519 signature with the exported public key (see console output for metrics and `Signature verified with exported public key: true`).

### Assignment & SDK labeling
- Deterministic assignment lives in `backend/src/controllers/migration.controller.ts#getAssignment` and `backend/src/services/migrationStudioService.ts#assignArm`. The service hashes `user_identifier:placement_id:seed` and returns `arm`, `experiment_id`, `mirror_percent`, `mode`, and `assignment_ts` while logging a hashed user fingerprint for privacy.
- SDKs call `POST /api/v1/migration/assign` before an impression request:

	```bash
	curl -X POST "${API_BASE}/api/v1/migration/assign" \
		-H 'Content-Type: application/json' \
		-d '{
			"user_identifier": "ifa:EAFCF9A7-CE06-4C82-ABF0-087DE0F2622F",
			"placement_id": "placement-iron-interstitial"
		}'
	```

	Sample response:

	```json
	{
		"success": true,
		"data": {
			"has_experiment": true,
			"experiment_id": "exp_48fd41b8",
			"arm": "test",
			"mirror_percent": 10,
			"assignment_ts": "2025-11-12T21:33:40.814Z",
			"mode": "shadow",
			"feature_flag_source": "publisher"
		}
	}
	```

- Clients forward the labels on every RTB auction request by embedding them under `signal.migration`:

	```json
	{
		"requestId": "ios-0f5c...",
		"placementId": "placement-iron-interstitial",
		"adFormat": "interstitial",
		"floorCpm": 0.0,
		"signal": {
			"migration": {
				"experiment_id": "exp_48fd41b8",
				"arm": "test",
				"assignment_ts": "2025-11-12T21:33:40.814Z",
				"mirror_percent": 10,
				"mode": "shadow",
				"feature_flag_source": "publisher"
			}
		}
	}
	```

- Feature flags gate every assignment: `MigrationStudioService#getEffectiveFeatureFlags` resolves the active scope (placement → app → publisher). Responses include `feature_flag_source` so SDK telemetry can confirm rollout provenance before forwarding requests.

- The RTB orchestrator (`backend/src/services/rtb/orchestrator.ts`) now consumes these labels, annotates Prometheus metrics (`auction_latency_seconds`, `rtb_wins_total`, `rtb_no_fill_total`, `rtb_errors_total`), and echoes the metadata back in the winning payload (`payload.migration`). Shadow mode short-circuits delivery, persisting virtual bids via `migration_shadow_outcomes` while mirroring mode continues to serve the creative and records identical telemetry for post-analysis.

## Guardrails and safety
- Hard caps per placement: max mirror %; latency budget; revenue floor; pause on breach.
- Kill switch: one-click disable for an experiment; immediate stop.
- Privacy: no additional PII; reuse consent flags; honor ATT/GDPR/CCPA flows.

## Reporting and verification
- Side-by-side tables and charts: eCPM, fill, latency p95, IVT rate, net revenue.
- Statistical analysis: CUPED/stratified comparisons; confidence intervals and MDE callouts.
- Verifiable artifacts: signed JSON report with references to underlying Ed25519 canonical records.
- Shareable read-only link with time-bounded token.

## Observability
- New Prometheus labels for control vs test; dashboards for experiment-level RED and lift.
- Alerts on guardrail breaches (latency, timeout spikes, revenue underperformance).

## Rollout plan
1) Build control plane and UI scaffolding with dry-run mode (no traffic shift).
2) Integrate assignment labels; compute reports from logs only.
3) Enable small mirror percent (≤5%) on staging publishers; validate metrics & guardrails.
4) Expand adapter coverage; add incumbent API import connectors.
5) GA: enable for selected publishers with success playbook.

## Acceptance criteria (summary)
- Create experiment, import mapping, mirror 10% on a placement, and see live comparison safely.
- No SLA regressions; guardrails trigger correctly; kill switch works within seconds.
- Reports match backend counters within 1%; signed JSON export validates.

## Related
- DEVELOPMENT_TODO_CHECKLIST Section 8
- Backend metrics: auction_latency_seconds, rtb_* counters
- Transparency: Ed25519 canonicalization and verification tooling
- Runbook: [Migration Studio Guardrails](../../runbooks/migration-studio-guardrails.md)

---

## Implementation Architecture

### Postgres Guardrail Snapshots

Migration Studio uses periodic snapshots to aggregate experiment metrics for real-time guardrail evaluation and reporting.

**Primary Tables**:
- `migration_shadow_outcomes`: Raw auction outcomes (shadow recorder).
- `migration_guardrail_snapshots`: Aggregated snapshots (impressions, fills, revenue, latency p50/p95, error rates).

**Performance Strategy**:
- **BRIN Indexes**: All time-series tables use Block Range Indexes on `created_at` for efficient range scans.
- **Sampling**: `sample_percentile_if_large()` is used for latency percentiles on datasets >100k rows to avoid sort-heavy `PERCENTILE_CONT`.
- **Unit Standard**: All revenue is stored in **micros** (1/1,000,000 USD) to ensure high precision across multi-currency networks.

**Snapshot Query Logic**:
```sql
SELECT 
  arm,
  COUNT(*) FILTER (WHERE status IN ('win', 'served')) AS impressions,
  COALESCE(SUM(bid_cpm * 1000), 0) AS revenue_micros,
  sample_percentile_if_large(experiment_id, arm, 0.95, 60) AS latency_p95_ms
FROM migration_shadow_outcomes
WHERE experiment_id = $1 AND created_at >= NOW() - interval '60 minutes'
GROUP BY arm;
```

### Prometheus Metrics Catalog

All metrics include `arm` (control|test) and `exp_id` labels for experiment tracking.

**Core Auction Metrics**:
- `auction_latency_seconds{arm, exp_id}` — Histogram (p50, p95, p99)
- `rtb_wins_total{adapter, arm, exp_id}` — Counter (wins by adapter and arm)
- `rtb_no_fill_total{arm, exp_id}` — Counter (no-fill events by arm)
- `rtb_errors_total{code, adapter, arm, exp_id}` — Counter (errors by taxonomy code)

**Guardrail Metrics**:
- `migration_guardrail_pauses_total{reason}` — Counter (auto-pause events: latency, error, fill)
- `migration_kills_total{reason}` — Counter (kill switch triggers: revenue, latency)

**Usage Examples**:
```promql
# eCPM comparison (control vs test)
sum by (arm) (rate(rtb_wins_total{exp_id="<exp_id>"}[5m]))

# Latency p95 by arm
histogram_quantile(0.95, sum by (arm, le) (rate(auction_latency_seconds_bucket{exp_id="<exp_id>"}[5m]))) * 1000

# Guardrail pause rate (last hour)
increase(migration_guardrail_pauses_total{exp_id="<exp_id>"}[1h])
```

### Guardrail Evaluation Flow

1. **Periodic Snapshot Capture**: Analytics pipeline aggregates hourly metrics into `migration_guardrail_snapshots` table
2. **On-Demand Evaluation**: `MigrationStudioService#evaluateGuardrails` queries last 6 hours of snapshots
3. **Threshold Checks**:
   - **Latency Budget**: Test arm p95 > `latency_budget_ms` → Critical violation
   - **Revenue Floor**: Test eCPM delta < `revenue_floor_percent` → Critical violation
   - **Error Rate**: Test error rate > `max_error_rate_percent` → Pause
4. **Auto-Remediation**:
   - **Pause**: Experiment status → `paused`, mode → `shadow`, event logged
   - **Kill**: Same as pause + Prometheus `migration_kills_total` incremented
5. **Notifications**: Prometheus alert fires → Alertmanager → PagerDuty/Slack

**Minimum Impressions**: Evaluation skipped if test arm < `min_impressions` (default: 1000)

### Report Generation & Verification

**Endpoint**: `GET /api/v1/migration/reports/:experimentId`

**Process**:
1. Query `migration_guardrail_snapshots` for last 14 days
2. Aggregate control/test metrics (impressions, fills, revenue, latency, IVT)
3. Calculate uplift percentages:
   - eCPM uplift: `((test_ecpm - control_ecpm) / control_ecpm) * 100`
   - Fill uplift: Similar calculation on fill rates
   - Latency delta: Test p95 - Control p95
4. Statistical significance: Simplified t-test approximation (production uses CUPED)
5. Projection: "If 100% test" scenario using control revenue baseline
6. Generate Ed25519 signed artifact via `generateSignedReportComparison`

**Signed Artifact Structure**:
```json
{
  "generated_at": "2025-11-12T22:15:00.000Z",
  "period": {
    "start": "2025-10-29T00:00:00.000Z",
    "end": "2025-11-12T00:00:00.000Z"
  },
  "sample_size": {
    "control_impressions": 45000,
    "test_impressions": 5000
  },
  "metrics": {
    "ecpm": {
      "label": "eCPM",
      "unit": "currency_cents",
      "control": 245.32,
      "test": 257.59,
      "uplift_percent": 4.99
    }
  },
  "signature": {
    "key_id": "migration-prod-2025",
    "algo": "ed25519",
    "payload_base64": "...",
    "signature_base64": "...",
    "public_key_base64": "..."
  }
}
```

**Verification**:
```bash
# CLI verifier (Node.js)
node backend/scripts/verifyMigrationReport.js report.json
```

### Baseline Backfill Procedure

When an experiment activates, populate 14-day control arm baseline for accurate guardrail evaluation.

**Script**: `backend/scripts/migrationBackfillBaseline.ts`

**Usage**:
```bash
DATABASE_URL="postgres://..." \
npx ts-node backend/scripts/migrationBackfillBaseline.ts <experiment_id>
```

**Process**:
1. Query experiment details (placement_id, activated_at)
2. Calculate date range: `activated_at - 14 days` to `activated_at`
3. Query Postgres `migration_shadow_outcomes` or `analytics_revenue_events` table
4. Aggregate daily snapshots (impressions, fills, revenue, latency, errors)
5. Insert into `migration_guardrail_snapshots` with `arm = 'control'`

**Note**: Script uses efficient indexed scans (BRIN) to aggregate baseline data.

### Grafana Dashboard

Dashboard UID: `migration-studio`
Path: `monitoring/grafana/migration-studio.json`

**Panels**:
- **Overview**: Wins/sec, Revenue uplift %, Guardrail pauses/kills (24h)
- **RED Metrics by Arm**: Rate (wins/no-fill), Errors (by code), Duration (p50/p95/p99 latency)
- **Side-by-Side Comparison**: Fill rate, eCPM proxy, latency comparison
- **Guardrail Status**: Pause/kill events by reason (1h rate)

**Variables**:
- `$experiment`: Dropdown populated from `label_values(rtb_wins_total, exp_id)`

**Annotations**:
- Guardrail pauses (orange)
- Kill switches (red)

**Access (Hetzner Production)**: `http://203.0.113.10:3001/d/apex-rtb-analytics`

### Alert Routing

All Migration Studio alerts route through Prometheus → Alertmanager.

**Runbook**: [Migration Studio Guardrails](../../runbooks/migration-studio-guardrails.md)

**Alert Summary**:
- `MigrationGuardrailPause` (Warning): Auto-pause triggered
- `MigrationKillSwitch` (Critical): Kill switch triggered
- `MigrationHighLatency` (Warning): Test arm p95 >500ms
- `MigrationRevenueDrop` (Critical): Test eCPM <85% of control
- `MigrationTestArmNoFill` (Warning): Test no-fill 20%+ higher

**Notification Channels**:
- Email: DevOps team (configured in Alertmanager)
- Slack: `#migration-studio-alerts` (requires Slack webhook configuration)
- Email: Publisher stakeholders (configurable per experiment)
