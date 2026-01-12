-- Add histogram-style revenue buckets to analytics_revenue_rollups
-- Motivation: preserve distribution shape without retaining raw events long-term.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '90s';

ALTER TABLE IF EXISTS analytics_revenue_rollups
  ADD COLUMN IF NOT EXISTS revenue_bucket_lt_1_count bigint NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS revenue_bucket_1_5_count bigint NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS revenue_bucket_gt_5_count bigint NOT NULL DEFAULT 0;
