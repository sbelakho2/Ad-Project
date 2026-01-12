-- Down migration: remove histogram-style revenue buckets

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '90s';

ALTER TABLE IF EXISTS analytics_revenue_rollups
  DROP COLUMN IF EXISTS revenue_bucket_lt_1_count,
  DROP COLUMN IF EXISTS revenue_bucket_1_5_count,
  DROP COLUMN IF EXISTS revenue_bucket_gt_5_count;
