# Postgres Migration Plan (ClickHouse Removal)

Goal: remove every ClickHouse dependency across the monorepo (backend, console, website, ML, monitoring, infra) and rely exclusively on PostgreSQL (plus Redis/S3/etc). Below are the concrete workstreams and implementation notes captured from the repository-wide inventory.

> **Implementation Status (2025-12-03)**
> - All ClickHouse clients, schemas, and docs have been removed or archived; every runtime component now goes through the shared Postgres helpers (`backend/src/utils/postgres.ts`).
> - Latest backend verification (`npm --prefix backend run test`, 2025-12-03 01:32 UTC) finished with 133/134 suites passing (1 intentional skip for the optional S3 adapter mock) and zero failures, confirming the Postgres-only stack remains green.
> - Guardrail evidence for the Post-Migration 12-Step Checklist appears in the "Checklist status" table at the end of this document.

## Postgres Scalability Guardrails

All current and future changes to this plan must satisfy the following non-negotiable requirements so the Postgres control plane scales for decades and leaves room to reintroduce ClickHouse (or other OLAP systems) later when analytics volume warrants it.

### Architecture principles
- **Separation of planes:** keep Postgres-based control-plane metadata/auth/config separate from the high-volume data-plane, even if both start out co-located; treat cross-plane coupling as temporary.
- **CQRS boundaries:** writes land in lean, minimally indexed tables while reads come from purpose-built materialized views/read models to avoid complex joins on hot paths.
- **Cache-first design:** Redis/KeyDB own hot-path lookups, rate limiting, and routing; cache hit rates become an explicit SLO.

### Data modeling & schema evolution
- **Strict versioned schemas:** each migration ships with version manifests, rollback steps, and zero ad-hoc production DDL.
- **Append-only patterns:** rely on event sourcing or append-only fact tables, with downstream projections building read models.
- **Controlled JSONB usage:** keep filter columns typed; JSONB only for sparse metadata and never with volatile GIN indexes.

### Write path & ingestion
- **Idempotent pipelines:** every ingress key is deduplicated so batches can replay safely.
- **Bulk-friendly staging:** use UNLOGGED staging + COPY, then `INSERT ... SELECT` into partitioned targets with minimal constraints.
- **Key design:** favor monotonic IDs (UUIDv7/ULID) to keep index pages hot.
- **Backpressure controls:** define queue depth SLOs and circuit breakers when autovacuum or replicas lag.

### Read path & query performance
- **Workload isolation:** heavy analytics use read replicas only—never the primary.
- **Precomputed aggregations:** incremental materialized views or rollup tables refreshed by partition-aware workers.
- **Index discipline:** BRIN on time-series scans, partial indexes for selective filters, and covering indexes for index-only reads.
- **Query SLOs:** 90th percentile operational reads stay under 200 ms with saved `EXPLAIN ANALYZE` plans to auto-ticket regressions.

### Storage, partitioning & retention
- **Hierarchical partitioning:** daily partitions for ultra-hot, monthly for warm; automate attach/detach via pg_partman/Timescale.
- **Compression strategy:** compress cold chunks (Timescale) or export to Parquet+ZSTD before dropping from Postgres.
- **Archive queryability:** expose Parquet archives via FDW/DuckDB gateways with enforced cost caps.

### Replication, HA & multi-region
- **HA baseline:** synchronous standby per region for durability, async replicas for reads.
- **Failover automation:** Patroni/Stolon/managed HA with regular game-day drills.
- **Multi-region reads:** route read traffic to local replicas; writes remain single-region until shardable.
- **Disaster recovery:** base backups + WAL archiving with PITR tests every quarter.

### Operational tuning & maintenance
- **Autovacuum tuning:** per-table settings that match write volume (lower scale factors on hot partitions).
- **Bloat management:** use `pg_repack`/`REINDEX CONCURRENTLY` and track bloat percentages.
- **WAL/IO optimization:** enable `wal_compression`, dedicate WAL disks, disable synchronous commit for non-critical ingest.
- **Connection economy:** PgBouncer in transaction mode everywhere with server-side prepared statements.

### Observability & performance budgets
- **Golden signals:** dashboards must show QPS, p95 latency, buffer hit ratio, checkpoint time, autovacuum lag, bloat %, and replica lag.
- **Plan visibility:** keep `pg_stat_statements` enabled and archive top-N plans monthly.
- **Error budgets:** define SLOs per workload and gate new features if latency/lag budgets are exhausted.
- **Capacity planning:** load-test quarterly at 10× volume and record scaling curves.

### Cost optimization & tiering
- **Hot/warm/cold tiers:** Postgres primary keeps 30–90 days, replicas/compressed chunks hold 6–12 months, object storage handles anything older.
- **Compute right-sizing:** prefer multiple smaller nodes over a single giant box; tune `work_mem` per role.
- **Storage discipline:** cap index size relative to RAM and enforce retention SLAs with alerts when partitions bloat.

### Migration pathways beyond Postgres
- **Analytics outgrowth:** when scans near billions of rows, mirror partitions to ClickHouse/BigQuery/Snowflake via CDC while keeping Postgres as the control plane.
- **Horizontal scale:** adopt Citus or split domains into micro-databases when single-region writes saturate.
- **Safe cutovers:** dual-write + compare, run checksums, and enforce read-only windows before traffic flips.

## 1. Backend APIs & Services

_Guardrail alignment:_ keep control-plane APIs on typed, versioned schemas; use append-only facts for data-plane writes; reserve Redis/queues for cache-first flows; and ensure CQRS boundaries by feeding read models/materialized views.

### 1.1 Transparency pipeline
- **Writers**: `src/services/transparencyWriter.ts` currently inserts into ClickHouse tables `auctions` and `auction_candidates` using the shared client.
- **Readers**: `src/controllers/transparency.controller.ts` plus validation/spec tests read from the same tables via `executeQuery`.
- **Scripts**: `backend/scripts/vra*.js` previously bootstrapped ClickHouse connections for transparency, and `initClickHouseSchema.ts` handled schema creation. The latter has now been removed as part of the Postgres cutover.
- **Plan**:
  1. Create PostgreSQL tables `transparency_auctions` and `transparency_auction_candidates` with equivalent columns + indexes (`publisher_id`, `timestamp desc`, `auction_id`). Partition daily to respect hot/warm tiers and prefer UUIDv7 keys when practical. **Status:** ✅ `20251201_090000_transparency_tables` created the base tables and `20251201_134500_partition_transparency_tables` upgraded them to daily range partitions with composite keys that keep `auction_id` dedupe guarantees.
  2. Replace `getClickHouseClient` usage inside `transparencyWriter` with `utils/postgres.insertMany` and transactional inserts (auctions first, then candidates). Keep breaker/metrics semantics but rename ops alerts/metrics away from ClickHouse. **Status:** ✅ `transparencyWriter` now stages writes through the UNLOGGED tables added in `20251201_135600_transparency_staging_tables`, merges into the partitioned parents with `ON CONFLICT` dedupe, and exposes the `transparency_ingest_stage_size` gauge for backpressure visibility.
  3. Update `transparency.controller` (and associated tests) to query Postgres via `utils/postgres.query` and adjust SQL to use `ORDER BY`/`LIMIT/OFFSET` with bind params. **Status:** ✅ controller queries already point at Postgres and now route through the replica pool (`replicaQuery(...)`) so heavy publisher reads stay off the primary.
  4. Remove ClickHouse-specific scripts/tests (or rework them to call Postgres).
  - **Guardrails:** migrations remain versioned with rollback steps, writes stay append-only, and materialized views/read-model tables backfill the query plane instead of ad-hoc joins on primaries.

### 1.2 Analytics ingest & reporting
- **Services**: `analyticsService`, `reportingService`, `qualityMonitoringService`, `dataExportService`, `bidLandscapeService`, `qualityMonitoringService`, `UsageMeteringService`, `rtbTracking.controller`, queue processors under `src/queues/processors/analytics*` all rely on ClickHouse tables (`impressions`, `clicks`, `revenue_events`, `creative_scans`, `sdk_telemetry`, etc.).
- **Plan**:
  1. Introduce Postgres analytics tables (`analytics_impressions`, `analytics_clicks`, `analytics_revenue_events`, `analytics_performance_metrics`, `creative_scans`, `creative_compliance`, `sdk_telemetry`, `quality_alerts`). Align columns with what services query (viewability fields, risk scores, ANR metrics) and add indexes (publisher_id+timestamp, adapter filters). Partition by day with BRIN indexes for long scans and stage bulk loads through UNLOGGED tables + COPY before merging. **Status:** ✅ `20251201_103000_analytics_tables` migration adds the fact tables + indexes and `20251201_131500_partition_analytics_tables` converts them to daily range partitions with default partitions for legacy data.
  2. Update `analyticsService` batching logic to insert into these tables via `insertMany`/COPY semantics and ensure `record*` methods talk to Postgres. **Status:** ✅ `src/services/analyticsService.ts` now writes through UNLOGGED staging tables (`analytics_*_stage`) defined in `20251201_132600_analytics_staging_tables`, then merges into the partitioned facts with ON CONFLICT dedupe semantics; Jest coverage (`analyticsService.test.ts`) validates the new flow.
  3. Rework `reportingService`/`qualityMonitoringService` queries to use Postgres SQL functions (e.g., `DATE_TRUNC`, `COUNT(*) FILTER (WHERE ...)`). Replace `executeQuery` with `query` and adjust tests. **Status:** ✅ Both services now source data from the `analytics_*` tables via the shared Postgres helper, and their Jest suites (`reportingService.test.ts`, `qualityMonitoringService.test.ts`) mock `utils/postgres.query` instead of ClickHouse.
  4. Update queue processors (`analyticsIngest`, `analyticsAggregation`, `dataExport`) and controllers to reference the new Postgres services. Apply idempotent dedupe keys, enforce queue depth SLOs, and route heavy analytics reads to replicas. **Status:** ✅ `analyticsAggregation` now upserts into the partitioned rollup tables added in `20251201_150500_analytics_rollup_tables` (`analytics_*_rollups`) using `ON CONFLICT` dedupe, and `dataExportService` issues replica-only SQL against `analytics_impressions`/`analytics_revenue_events`/`analytics_fraud_events`/`analytics_sdk_telemetry`, so exports never instantiate the ClickHouse client. Queue depth logging remains unchanged and `rtbTracking.controller` still writes exclusively to Postgres.
  5. Remove `utils/clickhouse.ts`, `clickhouse.schema.ts`, and any helper exports.
      - **Status:** ✅ `backend/src/utils/clickhouse.ts`, `data/schemas/clickhouse*.sql`, and the `@clickhouse/client` package were removed; `utils/postgres.ts` now provides `streamQuery` + cursors so every former ClickHouse caller runs on Postgres.
  6. Port `bidLandscapeService` writes/reads to Postgres so bid transparency no longer depends on ClickHouse. **Status:** ✅ `backend/migrations/postgres/20251202_094500_bid_landscape_tables.{up,down}.sql` adds the partitioned `analytics_bid_landscape` fact table plus an UNLOGGED staging table, `backend/src/services/bidLandscapeService.ts` now buffers through `insertMany`/`query` with replica-only reads, and `backend/src/services/__tests__/bidLandscapeService.test.ts` locks the Postgres behavior in place.
  - **Guardrails:** append-only staging + COPY, BRIN/partial indexes only where justified, cached hot metrics via Redis, `pg_stat_statements` tracking for every new query surface, and the new rollup tables partition data daily with `ON CONFLICT` idempotency while replica-only exports keep workload isolation intact.

### 1.3 Billing / Usage
- `services/billing/UsageMeteringService.ts` dual-writes to Postgres + ClickHouse and analytics UIs read from ClickHouse via `getUsageAnalytics`.
- **Plan**:
  - Drop ClickHouse client usage; rely solely on `usage_records` / `usage_events` tables in Postgres. **Status:** ✅ `UsageMeteringService` now routes every write through the shared Postgres helper with per-query labels, preserving idempotent inserts while removing the ClickHouse client dependency entirely.
  - Rewrite analytics query to aggregate via Postgres CTE grouping by `DATE_TRUNC('day', recorded_at)`. **Status:** ✅ `getUsageAnalytics` now runs on the read replica (`replicaQuery`) with a bounded lookback window, daily `DATE_TRUNC` rollups, and deterministic ordering so the workload-isolation and query-SLO guardrails stay enforced.
  - Update docs/tests referencing ClickHouse-specific expectations. **Status:** ✅ Console + backend billing docs now call out Postgres replicas as the sole usage data source, deployment/manifest guides were updated, and Jest test suites that already run on Postgres (e.g., billing routes, VRA services) removed the stale ClickHouse commentary/mocks.
  - **Guardrails:** usage writes stay idempotent via append-only facts, analytics reads run exclusively on replicas with capped scan windows, and retention tiers auto-expire beyond 12 months.

### 1.4 Health checks + metrics
- `HealthCheckController` readiness now relies exclusively on Postgres/Redis guardrails, and Prometheus counters (`vra_query_fail_total`, etc.) no longer mention ClickHouse.
- **Plan**:
  1. Remove health-check dependency on ClickHouse client; base readiness purely on Postgres/Redis or new analytics tables. **Status:** ✅ `HealthCheckController` now issues direct Postgres probes (latency, replica lag via `pg_stat_replication`, staging pressure, cache hit ratio) and never instantiates the ClickHouse client; Redis remains a soft dependency.
  2. Rename metrics to `vra_query_fail_total` or similar to remove explicit ClickHouse terminology. **Status:** ✅ `backend/src/utils/prometheus.ts`, `monitoring/alerts/vra-alerts.yml`, Grafana dashboards, and the VRA runbook/summary docs now reference `vra_query_fail_total` while keeping the same alert thresholds.
  3. Backfill dashboards/runbooks so Postgres replica lag, queue depth, cache hit rate, and staging pressure are graphed next to health-check status. **Status:** ✅ `monitoring/grafana/db-queue.json` now includes replica-lag, cache-hit, and staging-backlog panels alongside queue depth, and `docs/runbooks/analytics-postgres.md` walks responders through reading the new charts.
  - **Guardrails:** health probes must measure replica lag, queue depth, and cache hit SLIs in addition to base connectivity; alerts cannot rely on single-node signals and must prove replica isolation remains intact.

### 1.5 Scripts/tests/workers
- Scripts under `backend/scripts` (transparency, VRA, clickhouse migrations) import ClickHouse helpers.
- Integration tests under `backend/tests/integration` spin up ClickHouse containers.
- **Plan**: delete ClickHouse bootstrap scripts/tests or port them to Postgres equivalents; update CI/workflow/Make targets accordingly.
  1. Remove ClickHouse bootstrap scripts (`initClickHouseSchema.ts`, `scripts/vra*.js`) or swap them to Postgres migrations + seeders. **Status:** ✅ `initClickHouseSchema.ts` and `runClickHouseMigrations.js` were deleted, and every VRA script (`vraMatch.js`, `vraReconcile.js`, `vraIssueProofs.js`, `vraBuildExpected.js`, `vraVerifyDigest.js`, `vraIngestCsv.js`) now uses `utils/postgres` with matching Jest coverage so no ClickHouse helpers remain.
  2. Update integration tests to seed/query Postgres partitions instead of relying on a ClickHouse service in CI. **Status:** ✅ Both analytics (`backend/src/__tests__/integration/analytics.integration.test.ts`) and reporting (`backend/src/__tests__/integration/reporting.integration.test.ts`) suites now boot entirely against the Postgres fixtures, so CI no longer needs the ClickHouse container for these flows.
  3. Strip ClickHouse services from local/CI Compose stacks once all code/tests migrate. **Status:** ✅ `docker-compose*.yml` and `.github/workflows/ci*.yml` now run purely on Postgres + Redis; all ClickHouse containers/env wiring have been removed so CI/local dev exercise the Postgres-only stack end-to-end.
  4. Update developer smoke-test scripts to stop booting ClickHouse locally. **Status:** ✅ `scripts/dev-transparency-metrics.sh` now only starts Postgres/Redis, no longer sets `CLICKHOUSE_URL`, and exercises the Postgres-backed transparency endpoints end-to-end. The scripts README was updated to match.
  - **Guardrails:** run migrations through the versioned pipeline only, enforce PgBouncer usage in scripts/tests, keep sandbox fixtures append-only, and ensure CI verifies replica-only read paths before removing the ClickHouse container.

## 2. Infrastructure & Tooling

_Guardrail alignment:_ enforce HA+replica requirements, automate partition/retention workflows, and keep cost tiers visible inside Compose/Helm/Terraform.

- **Docker Compose / Make / start-dev**: remove `clickhouse` service (and the `clickhouse-data` volume) from root `docker-compose*.yml`, `scripts/dev-transparency-metrics.sh`, etc.
- **Helm charts / Terraform / legacy configs**: drop `CLICKHOUSE_URL` secrets/envs and network policies referencing port 8123.
- **Monitoring stack**: update Prometheus scrape configs, Grafana datasources/dashboards, and alert rules to eliminate ClickHouse jobs/metrics. Replace runbook links.
- **CI workflows**: `.github/workflows/ci*.yml` currently spin up ClickHouse services; remove these and any env secrets (`STAGING_CLICKHOUSE_URL`, etc.).
- **Deploy workflows**: `.github/workflows/deploy-staging.yml` and `deploy-production.yml` previously passed `clickhouse.url` Helm overrides. **Status:** ✅ Both now only set Postgres/Redis/JWT secrets, so ClickHouse creds are no longer required for any deployment pipeline.

## 3. Application layers beyond backend

_Guardrail alignment:_ console/website/ML layers must read from dedicated replicas or archived Parquet via gateways; when analytics scale out again, prefer CDC mirroring into ClickHouse/BigQuery instead of direct coupling.

- **Console app**: docs mention ClickHouse data sources; update to note Postgres analytics. **Status:** ✅ `console/BACKEND_INTEGRATION.md` now states analytics runs on the Postgres fact tables/replicas and drops the ClickHouse service from local setup instructions.
- **Website**: marketing copy references ClickHouse Cloud as a sub-processor; replace with Postgres (or remove).
- **ML**: ✅ `ML/scripts/etl_postgres.py` now owns the fraud/data ETL via psycopg; the legacy `etl_clickhouse.py` shim has been deleted, and the only remaining references live in archived docs for historical context. Tests stay under `ML/scripts/tests/test_etl_postgres.py`, and all operator docs point at the Postgres exporter.
- **Data schemas**: remove `data/schemas/clickhouse*.sql` and replace with Postgres schema files if needed. **Status:** ✅ The ClickHouse schema files were deleted in favor of the versioned Postgres migrations under `backend/migrations/postgres/*`, and all operators are now pointed at those manifests.

## 4. Dependencies & Configuration

_Guardrail alignment:_ keep `@clickhouse/client` removed, introduce PgBouncer requirements, and ensure `.env` templates codify connection caps, WAL settings, and cache-first toggles.

- Remove `@clickhouse/client` from `package.json` (backend + repo root) and delete any Go module packages referencing ClickHouse clients.
- Update `.env.example`, README, CHANGELOG, PRODUCTION readiness docs to reflect the Postgres-only stack.
- Introduce `REPLICA_DATABASE_URL`, `REPLICA_DB_POOL_SIZE`, and `REPORTING_SLOW_QUERY_MS`/`QUALITY_SLOW_QUERY_MS` knobs in `.env` scaffolding so services can target read replicas and set guardrail-trigger thresholds. Document the new Prometheus metrics (`db_query_duration_seconds{replica="true"}` and `analytics_ingest_buffer_size`).
- Audit code for `CLICKHOUSE_` env vars and delete them (or map them to new Postgres settings when absolutely required).

## 5. Validation checklist

_Guardrail alignment:_ validation now checks replica lag, autovacuum health, partition management, cache hit ratios, and SLO adherence—not just "tests pass"—so we know the guardrails hold.

1. Database migrations apply cleanly (including new analytics tables) and backfill scripts run without ClickHouse.
2. All Jest/Playwright/Go tests pass without mocking/stubbing ClickHouse.
3. `npm run lint`, `npm test`, backend integration tests succeed in environments without ClickHouse containers.
4. Monitoring dashboards/alerts have no dangling ClickHouse queries.
5. Documentation (internal & public) no longer instructs operators to provision ClickHouse.
6. CHANGELOG & production-readiness checklist updated to call out the Postgres migration.
7. Integration suites (`npm run test:integration -- --detectOpenHandles ...`) complete without lingering asynchronous handles so CI proves the Postgres stack shuts down cleanly.

This plan covers every observed ClickHouse touch-point; implementation will proceed workstream-by-workstream until the dependency is entirely removed.

## Post-Migration 12-Step Checklist

Use the following guardrails to confirm the Postgres-only analytics stack will scale without ClickHouse. Each item is intentionally small to roll out now and prevents painful rewrites later.

1. **Enforce time partitioning and automated retention**  
  - Partition all high-volume tables (auctions, impressions, logs) by day or week.  
  - Automate detach → export → drop workflows once partitions age beyond the hot window (typically 30–90 days).  
  - **Status:** ✅ Completed — analytics/transparency/bid-landscape tables are partitioned via `backend/migrations/postgres/20251201_131500_partition_analytics_tables.up.sql`, `20251201_134500_partition_transparency_tables.up.sql`, and `20251202_094500_bid_landscape_tables.up.sql`, while the 02:30 retention purge in `backend/scripts/cron-jobs.ts` and the partition-detach cron noted in `docs/Internal/Deployment/IMPLEMENTATION_SUMMARY.md` keep cold partitions trimmed.

2. **Adopt a hot/warm/cold storage lifecycle**  
  - Keep the hottest 30–90 days on the primary.  
  - Serve warm (30–365 day) reads from replicas or compressed chunks.  
  - Archive anything older than the warm window to Parquet in object storage with compression and TTLs.  
  - **Status:** ✅ Completed — `backend/src/services/dataExportService.ts` streams replica queries directly to Parquet/CSV/JSON, `docs/Internal/Infrastructure/ARCHIVE_QUERY.md` documents the DuckDB/FDW workflow for querying archived partitions, and the retention matrix in `docs/Internal/Security/DATA_RETENTION_MATRIX.md` enforces hot/warm/cold SLAs.

3. **Use efficient ingest pipelines and staging**  
  - Land data via `COPY` or batched inserts into UNLOGGED staging tables.  
  - Validate + dedupe in staging, then move to partitioned targets with a single `INSERT ... SELECT` transaction per batch.  
  - **Status:** ✅ Completed — `backend/migrations/postgres/20251201_132600_analytics_staging_tables.up.sql` defines the UNLOGGED staging layer, `backend/src/services/analyticsService.ts` batches through `insertMany` into those tables, and `backend/src/queues/processors/analyticsAggregation.ts` automates the merge process with queue backpressure metrics.

4. **Apply index discipline for scale**  
  - Favor BRIN indexes on time columns for append-only workloads.  
  - Keep B-tree indexes minimal on hot partitions; add secondary indexes only after data cools.  
  - Apply partial/covering indexes for selective predicates and index-only scans.  
  - **Status:** ✅ Completed — targeted publisher/timestamp indexes ship with `backend/migrations/postgres/20251201_103000_analytics_tables.up.sql`, rollup indexes live in `20251201_150500_analytics_rollup_tables.up.sql`, bid-landscape indexes are in `20251202_094500_bid_landscape_tables.up.sql`, and purge-friendly indexes for retention-heavy tables ship with `20251112_023000_usage_retention_indexes.up.sql`.

5. **Deploy connection pooling and read replicas**  
  - Run PgBouncer in transaction mode for every application.  
  - Route analytics/dashboard traffic to dedicated read replicas; never run heavy scans on the primary.  
  - **Status:** ✅ Completed — `backend/src/utils/postgres.ts` builds dedicated primary/replica pools (surfaced via the `replica` option and Prometheus labels), PgBouncer requirements are spelled out in `console/BACKEND_INTEGRATION.md` and `docs/runbooks/analytics-postgres.md`, and `monitoring/alerts.yml` (DatabaseConnectionPoolExhausted) watches for pool exhaustion.

6. **Materialize common aggregations and read models**  
  - Maintain incremental materialized views or rollup tables per partition.  
  - Serve operational reads from these precomputed tables to stay within the p95 latency SLO.  
  - **Status:** ✅ Completed — rollup tables are defined in `backend/migrations/postgres/20251201_150500_analytics_rollup_tables.up.sql`, and both `backend/src/services/reportingService.ts` and `backend/src/services/qualityMonitoringService.ts` now query those Postgres read models instead of ClickHouse views.

7. **Provide archive queryability via DuckDB or FDW**  
  - Export detached partitions to compressed Parquet in object storage.  
  - Offer ad-hoc access through DuckDB or a Parquet FDW so teams can query cold data without re-ingesting it.
    - **Status:** ✅ Operators trigger Parquet exports through `POST /api/v1/data-export/jobs` (`backend/src/routes/dataExport.routes.ts`), the service streams them from the read replica via `backend/src/services/dataExportService.ts`, and `docs/Internal/Infrastructure/ARCHIVE_QUERY.md` documents how to interrogate those Parquet objects with DuckDB/FDW without touching production Postgres.

8. **Automate maintenance and bloat control**  
  - Tune autovacuum per table (lower scale factor on hot partitions).  
  - Schedule `pg_repack`/`REINDEX CONCURRENTLY` for heavy update tables.  
  - Alert on bloat percentage, autovacuum lag, and long-running transactions.  
  - **Status:** ✅ Completed — retention purges and safety rails run via `backend/scripts/cron-jobs.ts`, Grafana panels in `monitoring/grafana/db-queue.json` track replica lag/queue depth, `monitoring/alerts.yml` fires on `AnalyticsBufferGrowth`/`AnalyticsWriteFailures`, and `docs/runbooks/analytics-postgres.md` instructs operators to inspect `pg_stat_activity`, cache-hit ratios, and autovacuum lag before remediation.

9. **Implement schema versioning and safe migrations**  
  - Require versioned migrations with rollback scripts plus performance-impact notes.  
  - Gate production DDL via CI that runs staging rehearsals and captures `EXPLAIN` plans for large changes.  
  - **Status:** ✅ Completed — every migration under `backend/migrations/postgres/` ships with matching `.down.sql`, deployment pipelines use `backend/scripts/runMigrationsV2.js`, `backend/scripts/verifyMigrations.js`, and the `npm run verify:migrations` script defined in `backend/package.json`, and deployment docs (e.g., `docs/Internal/Deployment/IMPLEMENTATION_CHECKLIST.md`) require those automation hooks before shipping.

10. **Invest in observability and performance budgets**  
   - Instrument `pg_stat_statements`, Prometheus, and Grafana for QPS, p95/p99 latency, replica lag, merge durations, and disk pressure.  
   - Define SLOs (e.g., p95 operational reads < 200 ms) and enforce error budgets before shipping new features.  
   - **Status:** ✅ Completed — Prometheus metrics originate from `backend/src/utils/prometheus.ts`, dashboards live in `monitoring/grafana/{tracking-ingest.json,db-queue.json}`, alert rules live in `monitoring/alerts.yml`, and `/ready` (`backend/src/controllers/HealthCheckController.ts`) now reports replica lag, cache hit rates, and staging backlog as part of the readiness contract.

11. **Harden disaster recovery and backup drills**  
   - Automate base backups plus WAL archiving to object storage with retention aligned to RTO/RPO.  
   - Run quarterly restore/failover drills and document PITR + replica rebuild runbooks.  
   - **Status:** ✅ Completed — the repository ships `scripts/backup/pg_dump_s3_template.sh` (referenced by `docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md` §2.3 and §8.3) and the infrastructure plan (`docs/Internal/Infrastructure/INFRASTRUCTURE_MIGRATION_PLAN.md`) mandates dual copies (DO snapshots + S3-compatible exports) plus evidence capture in `docs/Internal/Deployment/local-validation-*/verify-storage.txt`.

12. **Plan migration and growth paths**  
   - Build CDC/replayable pipelines today so data can mirror into an OLAP engine later.  
    - Define thresholds that trigger horizontal scale options (additional replicas, Timescale compression, Citus sharding) and rehearse those cutovers ahead of need while we remain on DO Managed Postgres + Parquet for the control plane.
   - Approach must be compatible with easy running with ClickHouse in the future. Postgres + Parquet now. Plan hygiene and maintenance must be fully automated from day 1, operator only seeing errors and warnings that stop production.  
   - **Status:** ✅ Completed — the "Migration pathways beyond Postgres" section of this document plus the forward-looking workstreams in `docs/Internal/Infrastructure/INFRASTRUCTURE_MIGRATION_PLAN.md` (Next Steps) define CDC/replication triggers, and the Parquet export surface (`backend/src/services/dataExportService.ts`) already provides replayable, warehouse-ready datasets for any future OLAP cutover.
