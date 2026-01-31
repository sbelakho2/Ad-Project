# Database Retention & Optimization Analysis

## 1. Current State Analysis

### Tables

* **`analytics_impressions`**: Partitioned (daily). Contains high-volume impression data. Size: ~35MB (currently low due to recent cleanup). Columns: Mix of UUIDs, text, numeric, and JSONB (`meta`). Retention: 7 days (aggressive).
* **`analytics_revenue_events`**: Not partitioned. Contains revenue events. Columns: Similar to impressions but with more specific revenue fields. Retention: 30 days. Risk: Retention cleanup via `DELETE` causes table bloat requiring frequent `VACUUM`/`VACUUM FULL` to reclaim space.

### Data Efficiency

* **JSONB overhead**: The `meta` column in `analytics_impressions` stores repetitive keys (`os`, `country_code`, `device_type`, etc.) for every single row.
* **Text columns**: Some fields are stored as free-form text.
* **Missing aggregations**: Reporting relies on raw facts, forcing longer raw retention than necessary.

---

## 2. Recommendations for Data Reduction

### A. Partitioning `analytics_revenue_events` (High Priority)

**Problem**: The table is not partitioned. Retention cleanup (`DELETE`) is inefficient.
**Solution**: Convert to a partitioned table (Range by `observed_at`), similar to `analytics_impressions`.
**Benefit**: Instant cleanup via `DROP TABLE`, zero bloat, faster queries for recent data.

### B. Schema Optimization (Medium Priority)

**Problem**: `meta` column stores redundant keys.
**Solution**: Extract high-cardinality fields into typed columns.

1. **`country_code`**: `CHAR(2)` vs JSON string.
2. **`os`**: `ENUM` vs JSON string.
3. **`device_type`**: `ENUM`.
4. **`ad_format`**: `ENUM`.

**Proposed Schema Change**:

```sql
ALTER TABLE analytics_impressions 
ADD COLUMN country_code CHAR(2),
ADD COLUMN os os_type_enum,
ADD COLUMN device_type device_type_enum;
-- Then remove these keys from 'meta' during insert
```

### C. Implement Aggregation (Rollups) (Highest Impact)

**Problem**: Long-range reporting forces raw facts to be retained longer than necessary.
**Solution**: Daily rollup tables (`analytics_*_rollups`) + a periodic backfill job.

**Implemented Tables (Postgres)**:

* `analytics_impression_rollups`
* `analytics_click_rollups`
* `analytics_revenue_rollups`
* `analytics_metrics_rollups`

These are created by migration `backend/migrations/postgres/20251201_150500_analytics_rollup_tables.up.sql`.

**Computation (Backend)**:

* Backend runs rollups on a configurable schedule and lookback window.
* Reporting endpoints prefer rollups when present and fall back to raw facts when absent.

**Key Env Vars**:

* `ANALYTICS_ROLLUP_ENABLED` (default: true)
* `ANALYTICS_ROLLUP_REALTIME_CRON` (default: `0 * * * *` UTC; refreshes *today*)
* `ANALYTICS_ROLLUP_FINALIZE_CRON` (optional; if unset, falls back to `ANALYTICS_ROLLUP_CRON`)
* `ANALYTICS_ROLLUP_CRON` (default: `15 0 * * *` UTC; finalizes *yesterday*)
* `ANALYTICS_ROLLUP_LOOKBACK_DAYS` (default: 35; used for startup backfill only)

**New Retention Policy (post-rollups)**:

* Raw facts (partitioned): keep 14 days by default.
* Rollups (monthly partitions): keep 5 years by default (or longer; low volume).

**Impact**: Reduces storage footprint by ~90% for long-term data.

### D. Index Optimization

* **`idx_analytics_impressions_event_id`**: This random UUID index is expensive to maintain. If it's only used for deduplication during ingestion, consider alternatives. If used for lookups, keep it.

### E. ClickHouse-Free High-Volume Strategy (Highest Priority)

**Problem**: Removing ClickHouse requires Postgres to handle 10x higher ingestion and query loads for RTB and Migration Studio.
**Solution**: Use specialized Postgres features for ad-tech time-series data.

1. **BRIN Indexes**: Replace B-Tree with Block Range Indexes on `observed_at` columns. 
   - *Impact*: 99% smaller index size; lightning-fast range scans on sequential data.
2. **HyperLogLog (HLL)**: Use `postgresql-hll` for unique user counts (UVs).
   - *Impact*: Sub-millisecond cardinality estimation vs. expensive `COUNT(DISTINCT)`.
3. **T-Digest**: Use for p95/p99 latency calculations.
   - *Impact*: $O(1)$ percentile approximations vs. $O(N \log N)$ full sorts.
4. **Partition Pruning**: Ensure all Migration Studio snapshots use `observed_at` in the `WHERE` clause to trigger partition exclusion.

---

## 3. Implementation Plan

1. **Immediate**:
    * Apply rollup table migration to production.
    * Confirm rollups are populated for the last 30 days.
    * Once verified, reduce raw-fact retention (default 14 days) as needed.

2. **Short-term**:
    * Monitor rollup performance (today refresh + yesterday finalize) and verify rollups cover reporting windows.

3. **Long-term**:
    * Optimize columns (ENUMs, extract from JSONB).

