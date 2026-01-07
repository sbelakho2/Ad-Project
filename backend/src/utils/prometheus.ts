import { collectDefaultMetrics, register, Histogram, Counter, Gauge } from 'prom-client';

const collectDefaults = process.env.PROMETHEUS_COLLECT_DEFAULTS !== '0';

if (collectDefaults) {
  collectDefaultMetrics();
}

export const promRegister = register;

export async function getPrometheusMetrics(): Promise<string> {
  return register.metrics();
}

// HTTP request duration histogram (seconds) - RED metrics (Rate, Errors, Duration)
export const httpRequestDurationSeconds = new Histogram({
  name: 'http_request_duration_seconds',
  help: 'Duration of HTTP requests in seconds (p50, p95, p99 per route)',
  labelNames: ['method', 'route', 'status_code'] as const,
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
});

// HTTP request counter (for rate calculation)
export const httpRequestsTotal = new Counter({
  name: 'http_requests_total',
  help: 'Total number of HTTP requests',
  labelNames: ['method', 'route', 'status_code'] as const,
});

// RTB auction latency histogram (seconds)
export const auctionLatencySeconds = new Histogram({
  name: 'auction_latency_seconds',
  help: 'Latency of RTB auction decisions in seconds',
  labelNames: ['arm', 'exp_id'] as const,
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2, 5],
});

// Adapter latency histogram (seconds)
export const rtbAdapterLatencySeconds = new Histogram({
  name: 'rtb_adapter_latency_seconds',
  help: 'Latency per RTB adapter in seconds',
  labelNames: ['adapter'] as const,
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2],
});

// Adapter timeout counter
export const rtbAdapterTimeoutsTotal = new Counter({
  name: 'rtb_adapter_timeouts_total',
  help: 'Total adapter timeouts',
  labelNames: ['adapter'] as const,
});

// Wins and no-fill
export const rtbWinsTotal = new Counter({
  name: 'rtb_wins_total',
  help: 'Total number of RTB auction wins by adapter',
  labelNames: ['adapter', 'arm', 'exp_id'] as const,
});

export const rtbNoFillTotal = new Counter({
  name: 'rtb_no_fill_total',
  help: 'Total number of RTB auctions resulting in no fill',
  labelNames: ['arm', 'exp_id'] as const,
});

// RTB error taxonomy counter
export const rtbErrorsTotal = new Counter({
  name: 'rtb_errors_total',
  help: 'Total number of RTB errors by code and adapter',
  labelNames: ['code', 'adapter', 'arm', 'exp_id'] as const,
});

// Database query duration histogram (seconds)
export const dbQueryDurationSeconds = new Histogram({
  name: 'db_query_duration_seconds',
  help: 'Duration of PostgreSQL queries in seconds',
  labelNames: ['operation', 'replica'] as const,
  buckets: [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2],
});

// Error counter
export const errorCounter = new Counter({
  name: 'app_errors_total',
  help: 'Total number of application errors',
  labelNames: ['type'] as const,
});

// Tracking-specific counters (public endpoints)
export const trackingRateLimitedTotal = new Counter({
  name: 'tracking_rate_limited_total',
  help: 'Total number of tracking requests that were rate limited',
});

export const trackingBlockedTotal = new Counter({
  name: 'tracking_blocked_total',
  help: 'Total number of tracking requests blocked by WAF/UA rules',
  labelNames: ['reason'] as const,
});

export const trackingHeadTotal = new Counter({
  name: 'tracking_head_total',
  help: 'Total number of HEAD requests to tracking endpoints',
});

// --- VRA pilot‑gate gauges (used by alerts; may be populated best‑effort) ---
export const vraCoveragePercent = new Gauge({
  name: 'vra_coverage_percent',
  help: 'VRA coverage as a percent (0..100)',
  labelNames: ['scope'] as const,
});

export const vraVariancePercent = new Gauge({
  name: 'vra_variance_percent',
  help: 'VRA unexplained variance as a percent (0..100)',
  labelNames: ['scope'] as const,
});

// Analytics ingestion (queue-based) metrics
export const analyticsEventsEnqueuedTotal = new Counter({
  name: 'analytics_events_enqueued_total',
  help: 'Total analytics events enqueued for ingestion',
  labelNames: ['kind'] as const,
});

export const analyticsEventsWrittenTotal = new Counter({
  name: 'analytics_events_written_total',
  help: 'Total analytics events successfully written to Postgres analytics tables',
  labelNames: ['kind'] as const,
});

export const analyticsEventsFailedTotal = new Counter({
  name: 'analytics_events_failed_total',
  help: 'Total analytics events failed to write to Postgres analytics tables',
  labelNames: ['kind'] as const,
});

export const analyticsIngestBufferGauge = new Gauge({
  name: 'analytics_ingest_buffer_size',
  help: 'Number of analytics events currently buffered before staging insert',
  labelNames: ['kind'] as const,
});

export const transparencyIngestStageGauge = new Gauge({
  name: 'transparency_ingest_stage_size',
  help: 'Rows currently pending merge inside transparency staging tables',
  labelNames: ['kind'] as const,
});

// Billing and Stripe-related counters
export const billingUsageLimitExceededTotal = new Counter({
  name: 'billing_usage_limit_exceeded_total',
  help: 'Total number of times an organization exceeded usage limits',
  labelNames: ['org_id'] as const,
});

export const stripeUsageSyncSuccessTotal = new Counter({
  name: 'stripe_usage_sync_success_total',
  help: 'Total number of successful Stripe usage sync operations',
  labelNames: ['org_id'] as const,
});

export const stripeUsageSyncFailuresTotal = new Counter({
  name: 'stripe_usage_sync_failures_total',
  help: 'Total number of failed Stripe usage sync operations',
  labelNames: ['org_id', 'reason'] as const,
});

// Migration Studio guardrail counters
export const migrationGuardrailPausesTotal = new Counter({
  name: 'migration_guardrail_pauses_total',
  help: 'Total number of experiment auto-pauses triggered by guardrails',
  labelNames: ['reason'] as const,
});

export const migrationKillsTotal = new Counter({
  name: 'migration_kills_total',
  help: 'Total number of experiment kill switches triggered by severe violations',
  labelNames: ['reason'] as const,
});

export const migrationGuardrailSnapshotsGenerated = new Counter({
  name: 'migration_guardrail_snapshots_generated_total',
  help: 'Total number of guardrail snapshots generated by the aggregation job',
  labelNames: ['arm'] as const,
});

// Authentication attempt counter
export const authAttemptsTotal = new Counter({
  name: 'auth_attempts_total',
  help: 'Total number of authentication/login attempts',
  labelNames: ['outcome'] as const, // outcome: success | failure | twofa_required
});

// 2FA events counter
export const twofaEventsTotal = new Counter({
  name: 'twofa_events_total',
  help: 'Total number of 2FA events',
  labelNames: ['event', 'outcome'] as const, // event: enroll|verify|login2fa|regen|disable
});

// --- VRA (Verifiable Revenue Auditor) metrics ---

// Duration of VRA read-model queries by operation (seconds)
export const vraQueryDurationSeconds = new Histogram({
  name: 'vra_query_duration_seconds',
  help: 'Duration of VRA read-model queries (Postgres replicas) in seconds by operation',
  labelNames: ['op', 'success'] as const, // op: overview|deltas_count|deltas_list|monthly_digest
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
});

// Times when VRA fell back to empty/0 results due to read-model failures
export const vraQueryFailTotal = new Counter({
  name: 'vra_query_fail_total',
  help: 'Total number of VRA fallbacks to safe empty results due to analytics read-model failures',
  labelNames: ['op'] as const,
});

// Count of empty results returned (helps monitor coverage/fill without errors)
export const vraEmptyResultsTotal = new Counter({
  name: 'vra_empty_results_total',
  help: 'Total number of VRA queries returning empty results (by operation)',
  labelNames: ['op'] as const,
});

// Dispute acknowledgements in shadow mode
export const vraDisputeShadowAcksTotal = new Counter({
  name: 'vra_dispute_shadow_acks_total',
  help: 'Total number of shadow-mode dispute acknowledgements',
  labelNames: ['network'] as const,
});

// Disputes actually created (non-shadow)
export const vraDisputesCreatedTotal = new Counter({
  name: 'vra_disputes_created_total',
  help: 'Total number of disputes created when shadow mode is disabled',
  labelNames: ['network'] as const,
});

// Dispute lifecycle state transitions
export const vraDisputeStateTransitionsTotal = new Counter({
  name: 'vra_dispute_state_transitions_total',
  help: 'Total number of dispute state transitions',
  labelNames: ['from', 'to', 'network'] as const,
});

// Dispute resolution duration
export const vraDisputeResolutionDurationSeconds = new Histogram({
  name: 'vra_dispute_resolution_duration_seconds',
  help: 'Time from dispute submission to resolution',
  labelNames: ['network', 'outcome'] as const,
  buckets: [3600, 86400, 259200, 604800, 1209600, 2592000], // 1h, 1d, 3d, 7d, 14d, 30d
});

// Ingestion: parsed rows and loads
export const vraStatementsRowsParsedTotal = new Counter({
  name: 'vra_statements_rows_parsed_total',
  help: 'Total number of statement rows parsed from network reports',
  labelNames: ['network', 'schema_ver'] as const,
});

export const vraStatementsLoadsTotal = new Counter({
  name: 'vra_statements_loads_total',
  help: 'Total number of successful statement loads (raw and normalized)',
  labelNames: ['network', 'phase'] as const, // phase: raw|norm
});

export const vraStatementsLoadFailuresTotal = new Counter({
  name: 'vra_statements_load_failures_total',
  help: 'Total number of failed statement loads',
  labelNames: ['network', 'phase', 'reason'] as const,
});

// Expected Builder metrics
export const vraExpectedSeenTotal = new Counter({
  name: 'vra_expected_seen_total',
  help: 'Total number of receipts seen by Expected Builder (rows read from PG transparency_receipts)',
});

export const vraExpectedWrittenTotal = new Counter({
  name: 'vra_expected_written_total',
  help: 'Total number of expected rows written (or would be written in dry-run) to recon_expected',
});

export const vraExpectedSkippedTotal = new Counter({
  name: 'vra_expected_skipped_total',
  help: 'Total number of receipts skipped (already existed or no matching paid event)',
});

export const vraExpectedBuildDurationSeconds = new Histogram({
  name: 'vra_expected_build_duration_seconds',
  help: 'Duration of Expected Builder run in seconds',
  labelNames: ['outcome'] as const, // outcome: success|empty|dry_run|error
  buckets: [0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60],
});

// Matching engine metrics
export const vraMatchDurationSeconds = new Histogram({
  name: 'vra_match_duration_seconds',
  help: 'Duration of matching engine runs in seconds',
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
});

export const vraMatchCandidatesTotal = new Counter({
  name: 'vra_match_candidates_total',
  help: 'Total number of candidate pairs evaluated by the matching engine',
});

export const vraMatchAutoTotal = new Counter({
  name: 'vra_match_auto_total',
  help: 'Total number of matches auto-accepted (confidence >= threshold)',
});

export const vraMatchReviewTotal = new Counter({
  name: 'vra_match_review_total',
  help: 'Total number of matches sent for review (min_conf <= confidence < auto threshold)',
});

export const vraMatchUnmatchedTotal = new Counter({
  name: 'vra_match_unmatched_total',
  help: 'Total number of statements that remained unmatched',
});

// Exact-key matches (confidence 1.0 via explicit identifiers)
export const vraMatchExactTotal = new Counter({
  name: 'vra_match_exact_total',
  help: 'Total number of exact-key matches (confidence 1.0)',
});

// Review matches persisted to the Postgres recon_match_review table
export const vraMatchReviewPersistedTotal = new Counter({
  name: 'vra_match_review_persisted_total',
  help: 'Total number of review matches persisted to the Postgres recon_match_review table',
});

// Reconcile & Delta Classification metrics
export const vraReconcileDurationSeconds = new Histogram({
  name: 'vra_reconcile_duration_seconds',
  help: 'Duration of reconcile/delta classification runs in seconds',
  buckets: [0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60],
});

export const vraReconcileRowsTotal = new Counter({
  name: 'vra_reconcile_rows_total',
  help: 'Total number of rows emitted into recon_deltas by kind',
  labelNames: ['kind'] as const,
});

// Dispute kit builder metrics
export const vraDisputeKitsBuiltTotal = new Counter({
  name: 'vra_dispute_kits_built_total',
  help: 'Total number of dispute kits built',
  labelNames: ['network'] as const,
});

export const vraDisputeKitFailuresTotal = new Counter({
  name: 'vra_dispute_kit_failures_total',
  help: 'Total number of dispute kit build failures',
  labelNames: ['reason'] as const,
});

// Proofs issuance metrics
export const vraProofsIssuanceDurationSeconds = new Histogram({
  name: 'vra_proofs_issuance_duration_seconds',
  help: 'Duration of proofs issuance runs in seconds',
  buckets: [0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60],
});

export const vraProofsVerifyFailuresTotal = new Counter({
  name: 'vra_proofs_verify_failures_total',
  help: 'Total number of verification failures for proofs',
});

export const vraProofsCoveragePct = new Gauge({
  name: 'vra_proofs_coverage_pct',
  help: 'Coverage percentage of receipts included in daily roots (0..100)',
  labelNames: ['day'] as const,
});
