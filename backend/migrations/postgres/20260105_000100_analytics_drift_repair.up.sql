-- Repair analytics schema drift (ensure ingestion tables/columns exist)
--
-- Goals:
-- - Ensure staging tables exist for ingestion.
-- - Ensure commonly used columns exist (meta/metadata/is_test_mode).
-- - Add non-unique supporting indexes for dedupe queries.
--
-- Notes:
-- - Intentionally avoids destructive changes and avoids repartitioning.
-- - Uses non-unique indexes (not UNIQUE) to minimize failure risk on existing data.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- 1) Add missing columns (idempotent)

ALTER TABLE IF EXISTS analytics_impressions
  ADD COLUMN IF NOT EXISTS is_test_mode boolean NOT NULL DEFAULT false;
ALTER TABLE IF EXISTS analytics_impressions
  ADD COLUMN IF NOT EXISTS meta jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE IF EXISTS analytics_clicks
  ADD COLUMN IF NOT EXISTS is_test_mode boolean NOT NULL DEFAULT false;
ALTER TABLE IF EXISTS analytics_clicks
  ADD COLUMN IF NOT EXISTS meta jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE IF EXISTS analytics_revenue_events
  ADD COLUMN IF NOT EXISTS is_test_mode boolean NOT NULL DEFAULT false;
ALTER TABLE IF EXISTS analytics_revenue_events
  ADD COLUMN IF NOT EXISTS reconciliation_status text NOT NULL DEFAULT 'pending';
ALTER TABLE IF EXISTS analytics_revenue_events
  ADD COLUMN IF NOT EXISTS metadata jsonb NOT NULL DEFAULT '{}'::jsonb;

-- 2) Ensure default partitions exist if the tables are partitioned
DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM pg_partitioned_table pt
    JOIN pg_class c ON c.oid = pt.partrelid
    WHERE c.relname = 'analytics_impressions'
  ) THEN
    EXECUTE 'CREATE TABLE IF NOT EXISTS analytics_impressions_p_default PARTITION OF analytics_impressions DEFAULT';
  END IF;

  IF EXISTS (
    SELECT 1
    FROM pg_partitioned_table pt
    JOIN pg_class c ON c.oid = pt.partrelid
    WHERE c.relname = 'analytics_clicks'
  ) THEN
    EXECUTE 'CREATE TABLE IF NOT EXISTS analytics_clicks_p_default PARTITION OF analytics_clicks DEFAULT';
  END IF;

  IF EXISTS (
    SELECT 1
    FROM pg_partitioned_table pt
    JOIN pg_class c ON c.oid = pt.partrelid
    WHERE c.relname = 'analytics_revenue_events'
  ) THEN
    EXECUTE 'CREATE TABLE IF NOT EXISTS analytics_revenue_events_p_default PARTITION OF analytics_revenue_events DEFAULT';
  END IF;
END$$;

-- 3) Ensure unlogged staging tables exist
CREATE UNLOGGED TABLE IF NOT EXISTS analytics_impressions_stage (
  LIKE analytics_impressions INCLUDING DEFAULTS INCLUDING STATISTICS
);

CREATE UNLOGGED TABLE IF NOT EXISTS analytics_clicks_stage (
  LIKE analytics_clicks INCLUDING DEFAULTS INCLUDING STATISTICS
);

CREATE UNLOGGED TABLE IF NOT EXISTS analytics_revenue_events_stage (
  LIKE analytics_revenue_events INCLUDING DEFAULTS INCLUDING STATISTICS
);

TRUNCATE TABLE analytics_impressions_stage;
TRUNCATE TABLE analytics_clicks_stage;
TRUNCATE TABLE analytics_revenue_events_stage;

-- 4) Supporting indexes (idempotent; non-unique)
-- Helps NOT EXISTS dedupe/merge in the ingestion code.
CREATE INDEX IF NOT EXISTS idx_analytics_impressions_observed_event
  ON analytics_impressions (observed_at, event_id);

CREATE INDEX IF NOT EXISTS idx_analytics_clicks_observed_event
  ON analytics_clicks (observed_at, event_id);

CREATE INDEX IF NOT EXISTS idx_analytics_revenue_events_observed_event
  ON analytics_revenue_events (observed_at, event_id);
