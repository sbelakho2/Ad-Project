# Analytics Postgres Pipeline Runbook

Last updated: 2025-12-02  
Owner: Platform Engineering

Purpose  
- Provide a lightweight response checklist when analytics ingestion alerts fire.  
- Ensure the Postgres-only analytics stack (staging tables + rollups) stays within the Post-Migration 12-Step guardrails.

Rollups
- Rollup tables: `analytics_impression_rollups`, `analytics_click_rollups`, `analytics_revenue_rollups`, `analytics_metrics_rollups`.
- Rollup computation is performed by the backend process on a schedule (env vars `ANALYTICS_ROLLUP_*`).
- Exports and most day-granularity reporting prefer rollups; if rollups are missing, exports fall back to an on-the-fly aggregation query.

## Key Dashboards & Metrics
- **Grafana › Tracking & Ingest (`tracking-ingest.json`)**
  - `analytics_events_enqueued_total` vs `analytics_events_written_total`
  - `analytics_events_failed_total` by kind
  - `analytics_ingest_buffer_size` per event kind (staging backlog)
- **Grafana › DB & Queue Health (`db-queue.json`)**
  - `db_query_duration_seconds` (label `operation=analytics_merge`)
  - `pg_stat_replication_{replay,write}_lag` → Replica lag stats in ms
  - `pg_stat_database_blks_hit` / `pg_stat_database_blks_read` → buffer cache hit %
  - `pg_stat_user_tables_n_live_tup` scoped to `*_stage` tables → staging backlog
  - `queue_depth` for `analytics_ingest`
- **/ready health check**
  - Shows Postgres replica lag, queue depth, cache hit rate.

## When `AnalyticsWriteFailures` Fires
1. Open Grafana → Tracking & Ingest dashboard. Confirm which `kind` is failing.
2. In the backend logs (`apps/backend`), search for `analyticsEventsFailedTotal` entries. Note the error payload.
3. Check staging tables for stuck rows:
   ```bash
   psql "$DATABASE_URL" -c "SELECT kind, COUNT(*) FROM analytics_events_stage GROUP BY 1;"
   ```
4. Inspect recent migrations or schema changes that could have broken inserts.
5. If failure is schema-related, pause ingest by toggling the feature flag `ANALYTICS_INGEST_ENABLED=false`, deploy, backfill staging rows, then re-enable.

## When `AnalyticsBufferGrowth` Fires
1. Grafana → Tracking & Ingest → "Buffer Size" panel: identify the dominant `kind`.
2. Validate worker throughput:
   ```bash
   heroku logs --app apex-analytics --tail | grep analyticsAggregation
   ```
   (substitute with your process manager if not on Heroku).
3. Check Postgres for long-running merges or locks:
   ```bash
   psql "$DATABASE_URL" -c "SELECT pid, query, now() - query_start AS age FROM pg_stat_activity WHERE query LIKE '%analytics_%merge%';"
   ```
4. If merges are stuck, capture `EXPLAIN (ANALYZE, BUFFERS)` for `analytics_merge_partitions` functions, cancel the offending PID, and rerun the merge job with reduced batch sizes (`ANALYTICS_MERGE_BATCH=25000`).

## When `AnalyticsPipelineBroken` Fires
1. Compare enqueue vs write rates. If enqueue > write, confirm Redis + queue workers are healthy (`queue_depth`, `queue_jobs_failed_total`).
2. Validate network connectivity between workers and Postgres (PgBouncer in transaction mode). Restart PgBouncer if connection exhaustion detected.
3. Inspect Postgres replica lag via `/ready` or directly:
   ```bash
   psql "$DATABASE_URL" -c "SELECT EXTRACT(EPOCH FROM replay_lag) AS lag_s FROM pg_stat_replication;"
   ```
4. If lag > 60s, redirect analytics reads to the primary temporarily and scale the read replica.
5. Resume pipeline and monitor `analytics_events_written_total` returning to parity with enqueued rate. Leave an incident note in `docs/runbooks/OPERATOR_CHECKLIST.md` (Weekly section).

## When Rollups Are Missing / Stale
1. Confirm the rollup tables exist in Postgres (migration `20251201_150500_analytics_rollup_tables`).
2. Confirm the backend rollup scheduler is enabled and running (check backend logs for `Analytics rollups:`).
3. If the rollup window is too small for your reporting range, increase `ANALYTICS_ROLLUP_LOOKBACK_DAYS` and restart the backend.

## Escalation
- If alerts remain firing after 30 minutes, page Platform Engineering via PagerDuty (Analytics channel).
- Capture:
  - Screenshot of Tracking & Ingest dashboard
  - `pg_stat_activity` sample
  - Relevant worker logs (ingest + aggregation)
- File a retrospective task referencing the Post-Migration 12-Step Checklist item #10 (observability) to capture any missing metrics/panels.
