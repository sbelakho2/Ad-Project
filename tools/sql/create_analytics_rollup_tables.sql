-- Create analytics rollup tables (Postgres) used for long-range reporting with small storage.
-- Safe to run multiple times.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '90s';

CREATE TABLE IF NOT EXISTS analytics_impression_rollups (
  bucket_date date NOT NULL,
  publisher_id text NOT NULL,
  app_id text NOT NULL DEFAULT '',
  ad_unit_id text NOT NULL DEFAULT '',
  adapter_id text NOT NULL DEFAULT '',
  country_code char(2) NOT NULL DEFAULT 'ZZ',
  impression_count bigint NOT NULL DEFAULT 0,
  filled_count bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
) PARTITION BY RANGE (bucket_date);

CREATE TABLE IF NOT EXISTS analytics_impression_rollups_p_default
  PARTITION OF analytics_impression_rollups DEFAULT;

CREATE INDEX IF NOT EXISTS idx_analytics_impression_rollups_pub_date
  ON analytics_impression_rollups (publisher_id, bucket_date DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_impression_rollups_adapter
  ON analytics_impression_rollups (adapter_id, bucket_date DESC);

CREATE TABLE IF NOT EXISTS analytics_click_rollups (
  bucket_date date NOT NULL,
  publisher_id text NOT NULL,
  app_id text NOT NULL DEFAULT '',
  ad_unit_id text NOT NULL DEFAULT '',
  adapter_id text NOT NULL DEFAULT '',
  country_code char(2) NOT NULL DEFAULT 'ZZ',
  click_count bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
) PARTITION BY RANGE (bucket_date);

CREATE TABLE IF NOT EXISTS analytics_click_rollups_p_default
  PARTITION OF analytics_click_rollups DEFAULT;

CREATE INDEX IF NOT EXISTS idx_analytics_click_rollups_pub_date
  ON analytics_click_rollups (publisher_id, bucket_date DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_click_rollups_adapter
  ON analytics_click_rollups (adapter_id, bucket_date DESC);

CREATE TABLE IF NOT EXISTS analytics_revenue_rollups (
  bucket_date date NOT NULL,
  publisher_id text NOT NULL,
  app_id text NOT NULL DEFAULT '',
  ad_unit_id text NOT NULL DEFAULT '',
  adapter_id text NOT NULL DEFAULT '',
  country_code char(2) NOT NULL DEFAULT 'ZZ',
  total_revenue numeric(24,8) NOT NULL DEFAULT 0,
  avg_revenue numeric(24,8) NOT NULL DEFAULT 0,
  max_revenue numeric(24,8) NOT NULL DEFAULT 0,
  revenue_event_count bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
) PARTITION BY RANGE (bucket_date);

CREATE TABLE IF NOT EXISTS analytics_revenue_rollups_p_default
  PARTITION OF analytics_revenue_rollups DEFAULT;

CREATE INDEX IF NOT EXISTS idx_analytics_revenue_rollups_pub_date
  ON analytics_revenue_rollups (publisher_id, bucket_date DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_revenue_rollups_adapter
  ON analytics_revenue_rollups (adapter_id, bucket_date DESC);

CREATE TABLE IF NOT EXISTS analytics_metrics_rollups (
  bucket_date date NOT NULL,
  publisher_id text NOT NULL,
  app_id text NOT NULL DEFAULT '',
  ad_unit_id text NOT NULL DEFAULT '',
  adapter_id text NOT NULL DEFAULT '',
  country_code char(2) NOT NULL DEFAULT 'ZZ',
  impression_count bigint NOT NULL DEFAULT 0,
  filled_count bigint NOT NULL DEFAULT 0,
  click_count bigint NOT NULL DEFAULT 0,
  total_revenue numeric(24,8) NOT NULL DEFAULT 0,
  ctr numeric(18,8) NOT NULL DEFAULT 0,
  fill_rate numeric(18,8) NOT NULL DEFAULT 0,
  ecpm numeric(24,8) NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
) PARTITION BY RANGE (bucket_date);

CREATE TABLE IF NOT EXISTS analytics_metrics_rollups_p_default
  PARTITION OF analytics_metrics_rollups DEFAULT;

CREATE INDEX IF NOT EXISTS idx_analytics_metrics_rollups_pub_date
  ON analytics_metrics_rollups (publisher_id, bucket_date DESC);
CREATE INDEX IF NOT EXISTS idx_analytics_metrics_rollups_adapter
  ON analytics_metrics_rollups (adapter_id, bucket_date DESC);
