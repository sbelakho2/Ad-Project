-- Migration: Optimize PERCENTILE_CONT queries for scale
-- 
-- BACKGROUND: PERCENTILE_CONT requires sorting all values which becomes O(n log n)
-- as traffic scales. At 1000 active experiments with 10k impressions/hour each,
-- the snapshot job exceeds its 15-minute window.
--
-- SOLUTION: Use t-digest or DDSketch algorithms for approximate percentiles.
-- For Postgres, we can:
-- 1. Pre-aggregate into histogram buckets
-- 2. Use the tdigest extension if available
-- 3. Sample-based approximation for large datasets
--
-- This migration:
-- 1. Creates a histogram table for latency distribution
-- 2. Adds indexes for efficient bucket aggregation
-- 3. Creates a function for approximate percentile from buckets

-- Pre-defined latency buckets (in ms) for histogram storage
-- Buckets: 0-10, 10-25, 25-50, 50-100, 100-200, 200-500, 500-1000, 1000+
CREATE TABLE IF NOT EXISTS migration_latency_histograms (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id UUID NOT NULL REFERENCES migration_experiments(id) ON DELETE CASCADE,
    arm TEXT NOT NULL CHECK (arm IN ('control', 'test')),
    bucket_min_ms INTEGER NOT NULL,
    bucket_max_ms INTEGER NOT NULL,  -- NULL means infinity
    sample_count BIGINT NOT NULL DEFAULT 0,
    bucket_hour TIMESTAMPTZ NOT NULL,  -- Hour bucket for time-based rollup
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(experiment_id, arm, bucket_min_ms, bucket_hour)
);

-- Index for efficient aggregation
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_latency_hist_experiment 
    ON migration_latency_histograms(experiment_id, arm, bucket_hour DESC);

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_latency_hist_hour 
    ON migration_latency_histograms(bucket_hour);

-- Function to calculate approximate percentile from histogram buckets
-- Uses linear interpolation within the bucket containing the target percentile
CREATE OR REPLACE FUNCTION approx_percentile_from_histogram(
    p_experiment_id UUID,
    p_arm TEXT,
    p_percentile NUMERIC,  -- e.g., 0.95 for p95
    p_hours_back INTEGER DEFAULT 6
) RETURNS NUMERIC AS $$
DECLARE
    v_total_count BIGINT;
    v_target_count NUMERIC;
    v_cumulative BIGINT := 0;
    v_bucket RECORD;
    v_result NUMERIC;
BEGIN
    -- Get total count
    SELECT COALESCE(SUM(sample_count), 0) INTO v_total_count
    FROM migration_latency_histograms
    WHERE experiment_id = p_experiment_id
      AND arm = p_arm
      AND bucket_hour >= NOW() - (p_hours_back || ' hours')::interval;
    
    IF v_total_count = 0 THEN
        RETURN NULL;
    END IF;
    
    v_target_count := v_total_count * p_percentile;
    
    -- Iterate through buckets in order
    FOR v_bucket IN (
        SELECT bucket_min_ms, COALESCE(bucket_max_ms, bucket_min_ms + 1000) as bucket_max_ms, 
               SUM(sample_count) as count
        FROM migration_latency_histograms
        WHERE experiment_id = p_experiment_id
          AND arm = p_arm
          AND bucket_hour >= NOW() - (p_hours_back || ' hours')::interval
        GROUP BY bucket_min_ms, bucket_max_ms
        ORDER BY bucket_min_ms
    ) LOOP
        IF v_cumulative + v_bucket.count >= v_target_count THEN
            -- Target percentile is in this bucket
            -- Linear interpolation within bucket
            v_result := v_bucket.bucket_min_ms + 
                (v_target_count - v_cumulative) / NULLIF(v_bucket.count, 0) * 
                (v_bucket.bucket_max_ms - v_bucket.bucket_min_ms);
            RETURN ROUND(v_result, 0);
        END IF;
        v_cumulative := v_cumulative + v_bucket.count;
    END LOOP;
    
    -- Fallback: return max bucket
    SELECT COALESCE(MAX(bucket_max_ms), 1000) INTO v_result
    FROM migration_latency_histograms
    WHERE experiment_id = p_experiment_id AND arm = p_arm;
    
    RETURN v_result;
END;
$$ LANGUAGE plpgsql STABLE;

-- Add comment
COMMENT ON FUNCTION approx_percentile_from_histogram IS 
    'Calculates approximate percentile from pre-aggregated histogram buckets. ' ||
    'Faster than PERCENTILE_CONT for large datasets.';

-- Create materialized view for recent experiment performance summary
-- This is refreshed periodically by the snapshot job
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_experiment_performance_summary AS
SELECT 
    experiment_id,
    arm,
    DATE_TRUNC('hour', bucket_hour) AS hour,
    SUM(sample_count) AS total_samples,
    SUM(CASE WHEN bucket_min_ms <= 100 THEN sample_count ELSE 0 END) AS samples_under_100ms,
    SUM(CASE WHEN bucket_min_ms <= 200 THEN sample_count ELSE 0 END) AS samples_under_200ms,
    SUM(CASE WHEN bucket_min_ms <= 500 THEN sample_count ELSE 0 END) AS samples_under_500ms
FROM migration_latency_histograms
WHERE bucket_hour >= NOW() - INTERVAL '24 hours'
GROUP BY experiment_id, arm, DATE_TRUNC('hour', bucket_hour)
WITH NO DATA;

-- Index for the materialized view
CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_perf_summary_pk 
    ON mv_experiment_performance_summary(experiment_id, arm, hour);

-- Schedule refresh of materialized view (will be done in application code)
COMMENT ON MATERIALIZED VIEW mv_experiment_performance_summary IS
    'Pre-aggregated experiment performance metrics. Refresh every 15 minutes.';

-- Add sampling option for large shadow_outcomes tables
-- This triggers sampling when row count exceeds threshold
CREATE OR REPLACE FUNCTION sample_percentile_if_large(
    p_experiment_id UUID,
    p_arm TEXT,
    p_percentile NUMERIC,
    p_window_minutes INTEGER DEFAULT 60,
    p_sample_threshold INTEGER DEFAULT 100000  -- Sample if > 100k rows
) RETURNS NUMERIC AS $$
DECLARE
    v_count BIGINT;
    v_result NUMERIC;
BEGIN
    -- Check row count
    SELECT COUNT(*) INTO v_count
    FROM migration_shadow_outcomes
    WHERE experiment_id = p_experiment_id
      AND arm = p_arm
      AND created_at >= NOW() - (p_window_minutes || ' minutes')::interval
      AND latency_ms IS NOT NULL;
    
    IF v_count > p_sample_threshold THEN
        -- SECURITY FIX: Avoid TABLESAMPLE BERNOULLI on multi-tenant tables.
        -- TABLESAMPLE samples the base table BEFORE filtering, which leads to 0 rows
        -- for small experiments. Instead, we use random() filtering after the index-based scan.
        SELECT PERCENTILE_CONT(p_percentile) WITHIN GROUP (ORDER BY latency_ms)
        INTO v_result
        FROM (
            SELECT latency_ms
            FROM migration_shadow_outcomes
            WHERE experiment_id = p_experiment_id
              AND arm = p_arm
              AND created_at >= NOW() - (p_window_minutes || ' minutes')::interval
              AND latency_ms IS NOT NULL
              AND random() < 0.1 -- Sample ~10% of the filtered results
        ) sampled;
    ELSE
        -- Exact percentile for smaller datasets
        SELECT PERCENTILE_CONT(p_percentile) WITHIN GROUP (ORDER BY latency_ms)
        INTO v_result
        FROM migration_shadow_outcomes
        WHERE experiment_id = p_experiment_id
          AND arm = p_arm
          AND created_at >= NOW() - (p_window_minutes || ' minutes')::interval
          AND latency_ms IS NOT NULL;
    END IF;
    
    RETURN v_result;
END;
$$ LANGUAGE plpgsql STABLE;

COMMENT ON FUNCTION sample_percentile_if_large IS
    'Calculates percentile with automatic sampling for large datasets to avoid query timeouts.';
