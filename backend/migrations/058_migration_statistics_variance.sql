-- Migration: Add sum_squares columns for proper statistical variance calculation
-- 
-- BACKGROUND: The previous implementation used hardcoded variance (impressions * 0.05)
-- which produced invalid statistical significance. To compute actual variance, we need
-- to track SUM(x²) alongside SUM(x) using the identity:
--   Var(X) = E[X²] - E[X]² = (Σx²/n) - (Σx/n)²
--
-- This migration adds:
-- 1. revenue_sum_squares: SUM(bid_cpm²) for eCPM variance
-- 2. Enables proper Welch's t-test for A/B comparisons

-- Add sum_squares column to guardrail snapshots
ALTER TABLE migration_guardrail_snapshots
    ADD COLUMN IF NOT EXISTS revenue_sum_squares NUMERIC(30,6) DEFAULT 0;

-- Add comment explaining the column
COMMENT ON COLUMN migration_guardrail_snapshots.revenue_sum_squares IS 
    'Sum of squared revenue values (bid_cpm²) for variance calculation. Var = (Σx² - (Σx)²/n) / (n-1)';

-- Add latency sum squares for latency variance (useful for SLO analysis)  
ALTER TABLE migration_guardrail_snapshots
    ADD COLUMN IF NOT EXISTS latency_sum_squares NUMERIC(30,6) DEFAULT 0;

COMMENT ON COLUMN migration_guardrail_snapshots.latency_sum_squares IS
    'Sum of squared latency values for latency variance calculation';

-- Add index to speed up variance queries
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_guardrail_snapshots_variance 
    ON migration_guardrail_snapshots(experiment_id, arm, captured_at DESC)
    INCLUDE (impressions, revenue_micros, revenue_sum_squares);

-- Add sum_squares tracking to shadow outcomes for real-time variance
ALTER TABLE migration_shadow_outcomes
    ADD COLUMN IF NOT EXISTS bid_cpm_squared NUMERIC(20,6) 
        GENERATED ALWAYS AS (CASE WHEN bid_cpm IS NOT NULL THEN bid_cpm * bid_cpm ELSE NULL END) STORED;

COMMENT ON COLUMN migration_shadow_outcomes.bid_cpm_squared IS
    'Pre-computed squared bid_cpm for efficient variance aggregation';

-- Create partial index for variance aggregation queries
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_shadow_outcomes_variance
    ON migration_shadow_outcomes(experiment_id, arm, created_at DESC)
    WHERE bid_cpm IS NOT NULL;
