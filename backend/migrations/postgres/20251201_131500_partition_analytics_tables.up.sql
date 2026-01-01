-- Convert analytics fact tables to partitioned layouts (daily range partitions)
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

DO $$
BEGIN
  IF to_regclass('analytics_impressions_legacy') IS NULL AND to_regclass('analytics_impressions') IS NOT NULL THEN
    EXECUTE 'ALTER TABLE analytics_impressions RENAME TO analytics_impressions_legacy';
  END IF;
  IF to_regclass('analytics_clicks_legacy') IS NULL AND to_regclass('analytics_clicks') IS NOT NULL THEN
    EXECUTE 'ALTER TABLE analytics_clicks RENAME TO analytics_clicks_legacy';
  END IF;
  IF to_regclass('analytics_revenue_events_legacy') IS NULL AND to_regclass('analytics_revenue_events') IS NOT NULL THEN
    EXECUTE 'ALTER TABLE analytics_revenue_events RENAME TO analytics_revenue_events_legacy';
  END IF;
END$$;

-- Ensure legacy tables have all columns required for the backfill.
-- This migration may run on top of older schemas created by legacy init scripts.
DO $$
BEGIN
  IF to_regclass('analytics_impressions_legacy') IS NOT NULL THEN
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS ad_unit_id text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS ad_format text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS country_code char(2) DEFAULT ''ZZ''';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS device_type text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS os text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS os_version text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS session_id text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS user_id text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS viewable boolean NOT NULL DEFAULT false';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS measurable boolean NOT NULL DEFAULT false';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS view_duration_ms integer NOT NULL DEFAULT 0';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS request_id text';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS meta jsonb NOT NULL DEFAULT ''{}''::jsonb';
    EXECUTE 'ALTER TABLE analytics_impressions_legacy ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now()';
  END IF;

  IF to_regclass('analytics_clicks_legacy') IS NOT NULL THEN
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS impression_id uuid';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS publisher_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS app_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS placement_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS adapter_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS adapter_name text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS click_url text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS country_code char(2) DEFAULT ''ZZ''';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS device_type text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS os text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS session_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS user_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS request_id text';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS time_to_click_ms integer';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS is_verified boolean NOT NULL DEFAULT false';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS is_test_mode boolean NOT NULL DEFAULT false';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS meta jsonb NOT NULL DEFAULT ''{}''::jsonb';
    EXECUTE 'ALTER TABLE analytics_clicks_legacy ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now()';
  END IF;

  IF to_regclass('analytics_revenue_events_legacy') IS NOT NULL THEN
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS revenue_currency char(3) NOT NULL DEFAULT ''USD''';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS revenue_original numeric(18,6)';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS exchange_rate numeric(10,6)';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS ecpm_usd numeric(18,6)';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS country_code char(2) DEFAULT ''ZZ''';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS ad_format text';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS os text';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS is_test_mode boolean NOT NULL DEFAULT false';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS reconciliation_status text NOT NULL DEFAULT ''pending''';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS metadata jsonb NOT NULL DEFAULT ''{}''::jsonb';
    EXECUTE 'ALTER TABLE analytics_revenue_events_legacy ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now()';
  END IF;
END$$;

CREATE TABLE IF NOT EXISTS analytics_impressions (
  id bigserial,
  event_id uuid NOT NULL,
  observed_at timestamptz NOT NULL,
  publisher_id text NOT NULL,
  app_id text,
  placement_id text,
  adapter_id text,
  adapter_name text,
  ad_unit_id text,
  ad_format text,
  country_code char(2) DEFAULT 'ZZ',
  device_type text,
  os text,
  os_version text,
  session_id text,
  user_id text,
  request_id text,
  status text,
  filled boolean NOT NULL DEFAULT false,
  viewable boolean NOT NULL DEFAULT false,
  measurable boolean NOT NULL DEFAULT false,
  view_duration_ms integer NOT NULL DEFAULT 0,
  latency_ms integer,
  revenue_usd numeric(18,6) NOT NULL DEFAULT 0,
  is_test_mode boolean NOT NULL DEFAULT false,
  meta jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (observed_at, id)
) PARTITION BY RANGE (observed_at);

CREATE TABLE IF NOT EXISTS analytics_clicks (
  id bigserial,
  event_id uuid NOT NULL,
  observed_at timestamptz NOT NULL,
  impression_id uuid,
  publisher_id text NOT NULL,
  app_id text,
  placement_id text,
  adapter_id text,
  adapter_name text,
  click_url text,
  country_code char(2) DEFAULT 'ZZ',
  device_type text,
  os text,
  session_id text,
  user_id text,
  request_id text,
  time_to_click_ms integer,
  is_verified boolean NOT NULL DEFAULT false,
  is_test_mode boolean NOT NULL DEFAULT false,
  meta jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (observed_at, id)
) PARTITION BY RANGE (observed_at);

CREATE TABLE IF NOT EXISTS analytics_revenue_events (
  id bigserial,
  event_id uuid NOT NULL,
  observed_at timestamptz NOT NULL,
  publisher_id text NOT NULL,
  app_id text,
  placement_id text,
  adapter_id text,
  adapter_name text,
  impression_id uuid,
  revenue_type text NOT NULL,
  revenue_usd numeric(18,6) NOT NULL DEFAULT 0,
  revenue_currency char(3) NOT NULL DEFAULT 'USD',
  revenue_original numeric(18,6),
  exchange_rate numeric(10,6),
  ecpm_usd numeric(18,6),
  country_code char(2) DEFAULT 'ZZ',
  ad_format text,
  os text,
  is_test_mode boolean NOT NULL DEFAULT false,
  reconciliation_status text NOT NULL DEFAULT 'pending',
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (observed_at, id)
) PARTITION BY RANGE (observed_at);

-- Default (catch-all) partitions so existing data has a home
CREATE TABLE IF NOT EXISTS analytics_impressions_p_default
  PARTITION OF analytics_impressions DEFAULT;
CREATE TABLE IF NOT EXISTS analytics_clicks_p_default
  PARTITION OF analytics_clicks DEFAULT;
CREATE TABLE IF NOT EXISTS analytics_revenue_events_p_default
  PARTITION OF analytics_revenue_events DEFAULT;

-- Unique constraints scoped with partition key
CREATE UNIQUE INDEX IF NOT EXISTS analytics_impressions_event_uq
  ON analytics_impressions (observed_at, event_id);
CREATE UNIQUE INDEX IF NOT EXISTS analytics_clicks_event_uq
  ON analytics_clicks (observed_at, event_id);
CREATE UNIQUE INDEX IF NOT EXISTS analytics_revenue_event_uq
  ON analytics_revenue_events (observed_at, event_id);

-- Re-create covering indexes on parent so partitions inherit the structure
CREATE INDEX IF NOT EXISTS idx_analytics_impressions_publisher_observed
  ON analytics_impressions (publisher_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_impressions_adapter_observed
  ON analytics_impressions (adapter_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_impressions_request
  ON analytics_impressions (request_id);

CREATE INDEX IF NOT EXISTS idx_analytics_clicks_publisher_observed
  ON analytics_clicks (publisher_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_clicks_adapter_observed
  ON analytics_clicks (adapter_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_clicks_impression
  ON analytics_clicks (impression_id);

CREATE INDEX IF NOT EXISTS idx_analytics_revenue_publisher_observed
  ON analytics_revenue_events (publisher_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_revenue_impression
  ON analytics_revenue_events (impression_id);
CREATE INDEX IF NOT EXISTS idx_analytics_revenue_type_observed
  ON analytics_revenue_events (revenue_type, observed_at DESC);

-- Backfill existing rows into new partitioned tables
DO $$
BEGIN
  IF to_regclass('analytics_impressions_legacy') IS NOT NULL THEN
    INSERT INTO analytics_impressions (
      id, event_id, observed_at, publisher_id, app_id, placement_id, adapter_id, adapter_name,
      ad_unit_id, ad_format, country_code, device_type, os, os_version, session_id, user_id,
      request_id, status, filled, viewable, measurable, view_duration_ms, latency_ms,
      revenue_usd, is_test_mode, meta, created_at
    )
    SELECT
      id, event_id, observed_at, publisher_id, app_id, placement_id, adapter_id, adapter_name,
      ad_unit_id, ad_format, country_code, device_type, os, os_version, session_id, user_id,
      request_id, status, filled, viewable, measurable, view_duration_ms, latency_ms,
      revenue_usd, is_test_mode, meta, created_at
    FROM analytics_impressions_legacy;
  END IF;

  IF to_regclass('analytics_clicks_legacy') IS NOT NULL THEN
    INSERT INTO analytics_clicks (
      id, event_id, observed_at, impression_id, publisher_id, app_id, placement_id, adapter_id,
      adapter_name, click_url, country_code, device_type, os, session_id, user_id, request_id,
      time_to_click_ms, is_verified, is_test_mode, meta, created_at
    )
    SELECT
      id, event_id, observed_at, impression_id, publisher_id, app_id, placement_id, adapter_id,
      adapter_name, click_url, country_code, device_type, os, session_id, user_id, request_id,
      time_to_click_ms, is_verified, is_test_mode, meta, created_at
    FROM analytics_clicks_legacy;
  END IF;

  IF to_regclass('analytics_revenue_events_legacy') IS NOT NULL THEN
    INSERT INTO analytics_revenue_events (
      id, event_id, observed_at, publisher_id, app_id, placement_id, adapter_id, adapter_name,
      impression_id, revenue_type, revenue_usd, revenue_currency, revenue_original, exchange_rate,
      ecpm_usd, country_code, ad_format, os, is_test_mode, reconciliation_status, metadata, created_at
    )
    SELECT
      id, event_id, observed_at, publisher_id, app_id, placement_id, adapter_id, adapter_name,
      impression_id, revenue_type, revenue_usd, revenue_currency, revenue_original, exchange_rate,
      ecpm_usd, country_code, ad_format, os, is_test_mode, reconciliation_status, metadata, created_at
    FROM analytics_revenue_events_legacy;
  END IF;

  DROP TABLE IF EXISTS analytics_impressions_legacy;
  DROP TABLE IF EXISTS analytics_clicks_legacy;
  DROP TABLE IF EXISTS analytics_revenue_events_legacy;
END$$;