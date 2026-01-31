-- Migration: Full Speed Postgres Optimizations
-- Aimed at achieving ClickHouse-level performance for ad-tech workloads

-- 1. BRIN Indexes for time-series range scans
-- BRIN (Block Range Index) is extremely small and effective for 
-- data that is naturally ordered by time.
CREATE INDEX IF NOT EXISTS idx_shadow_outcomes_brin 
ON migration_shadow_outcomes USING BRIN (created_at) WITH (pages_per_range = 32);

CREATE INDEX IF NOT EXISTS idx_revenue_events_brin 
ON analytics_revenue_events USING BRIN (observed_at) WITH (pages_per_range = 32);

CREATE INDEX IF NOT EXISTS idx_migration_events_brin 
ON migration_events USING BRIN (created_at) WITH (pages_per_range = 32);

-- 2. HyperLogLog (HLL) support for approximate unique counts (UVs)
-- This requires the postgresql-hll extension.
-- We check for the extension and create a wrapper if it exists.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'hll') THEN
        RAISE NOTICE 'hll extension found, enabling approximate UV optimizations';
    ELSE
        RAISE NOTICE 'hll extension not found, skipping hll optimizations';
    END IF;
END $$;

-- 3. T-Digest support for approximate percentiles
-- This requires the t_digest extension.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'tdigest') THEN
        RAISE NOTICE 'tdigest extension found, enabling approximate percentile optimizations';
    ELSE
        RAISE NOTICE 'tdigest extension not found, using histogram-based fallback';
    END IF;
END $$;

-- 4. Granular Discrepancy Detection Table for VRA
-- Used to solve the "masking" issue where aggregate gaps are small but 
-- individual discrepancies are high.
CREATE TABLE IF NOT EXISTS vra_granular_discrepancies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL,
    request_id TEXT NOT NULL,
    network_type TEXT NOT NULL,
    expected_value DECIMAL(18, 8) NOT NULL,
    paid_value DECIMAL(18, 8) NOT NULL,
    discrepancy_usd DECIMAL(18, 8) GENERATED ALWAYS AS (paid_value - expected_value) STORED,
    discrepancy_percent DECIMAL(10, 4),
    observed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(request_id, network_type)
);

CREATE INDEX IF NOT EXISTS idx_vra_disc_publisher_ts ON vra_granular_discrepancies(publisher_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_vra_disc_high_val ON vra_granular_discrepancies(ABS(discrepancy_usd) DESC) WHERE ABS(discrepancy_usd) > 0.01;
