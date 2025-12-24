-- Migration: Partition analytics_impressions by day for performance
-- Date: 2025-12-24
-- Purpose: Implement daily partitioning for analytics_impressions table to improve query performance and enable efficient data retention

-- Step 1: Rename existing table to _old
ALTER TABLE analytics_impressions RENAME TO analytics_impressions_old;

-- Step 2: Create new partitioned table
CREATE TABLE analytics_impressions (
    id bigint NOT NULL,
    event_id uuid NOT NULL DEFAULT gen_random_uuid(),
    observed_at timestamp with time zone NOT NULL DEFAULT now(),
    publisher_id uuid NOT NULL,
    app_id uuid,
    placement_id uuid,
    adapter_id text,
    adapter_name text,
    request_id uuid,
    status text,
    filled boolean NOT NULL DEFAULT false,
    latency_ms integer,
    revenue_usd numeric(18,6) NOT NULL DEFAULT 0,
    is_test_mode boolean NOT NULL DEFAULT false,
    meta jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    PRIMARY KEY (id, observed_at)
) PARTITION BY RANGE (observed_at);

-- Step 3: Create indexes on partitioned table
CREATE INDEX idx_analytics_impressions_adapter_observed 
    ON analytics_impressions (adapter_id, observed_at DESC);
CREATE INDEX idx_analytics_impressions_publisher_observed 
    ON analytics_impressions (publisher_id, observed_at DESC);
CREATE INDEX idx_analytics_impressions_event_id 
    ON analytics_impressions (event_id);

-- Step 4: Create sequence for ID generation (preserve existing sequence)
-- The sequence already exists, just alter the table to use it
ALTER TABLE analytics_impressions 
    ALTER COLUMN id SET DEFAULT nextval('analytics_impressions_id_seq'::regclass);

-- Step 5: Create partitions for existing data and future (7 days ahead)
-- Partition for 2025-12-17
CREATE TABLE analytics_impressions_2025_12_17 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-17 00:00:00+00') TO ('2025-12-18 00:00:00+00');

-- Partition for 2025-12-18
CREATE TABLE analytics_impressions_2025_12_18 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-18 00:00:00+00') TO ('2025-12-19 00:00:00+00');

-- Partition for 2025-12-19
CREATE TABLE analytics_impressions_2025_12_19 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-19 00:00:00+00') TO ('2025-12-20 00:00:00+00');

-- Partition for 2025-12-20
CREATE TABLE analytics_impressions_2025_12_20 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-20 00:00:00+00') TO ('2025-12-21 00:00:00+00');

-- Partition for 2025-12-21
CREATE TABLE analytics_impressions_2025_12_21 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-21 00:00:00+00') TO ('2025-12-22 00:00:00+00');

-- Partition for 2025-12-22
CREATE TABLE analytics_impressions_2025_12_22 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-22 00:00:00+00') TO ('2025-12-23 00:00:00+00');

-- Partition for 2025-12-23
CREATE TABLE analytics_impressions_2025_12_23 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-23 00:00:00+00') TO ('2025-12-24 00:00:00+00');

-- Partition for 2025-12-24 (today)
CREATE TABLE analytics_impressions_2025_12_24 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-24 00:00:00+00') TO ('2025-12-25 00:00:00+00');

-- Partition for 2025-12-25
CREATE TABLE analytics_impressions_2025_12_25 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-25 00:00:00+00') TO ('2025-12-26 00:00:00+00');

-- Partition for 2025-12-26
CREATE TABLE analytics_impressions_2025_12_26 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-26 00:00:00+00') TO ('2025-12-27 00:00:00+00');

-- Partition for 2025-12-27
CREATE TABLE analytics_impressions_2025_12_27 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-27 00:00:00+00') TO ('2025-12-28 00:00:00+00');

-- Partition for 2025-12-28
CREATE TABLE analytics_impressions_2025_12_28 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-28 00:00:00+00') TO ('2025-12-29 00:00:00+00');

-- Partition for 2025-12-29
CREATE TABLE analytics_impressions_2025_12_29 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-29 00:00:00+00') TO ('2025-12-30 00:00:00+00');

-- Partition for 2025-12-30
CREATE TABLE analytics_impressions_2025_12_30 PARTITION OF analytics_impressions
    FOR VALUES FROM ('2025-12-30 00:00:00+00') TO ('2025-12-31 00:00:00+00');

-- Step 6: Copy data from old table to new partitioned table
INSERT INTO analytics_impressions 
SELECT * FROM analytics_impressions_old;

-- Step 7: Verify data migration
DO $$
DECLARE
    old_count bigint;
    new_count bigint;
BEGIN
    SELECT COUNT(*) INTO old_count FROM analytics_impressions_old;
    SELECT COUNT(*) INTO new_count FROM analytics_impressions;
    
    IF old_count != new_count THEN
        RAISE EXCEPTION 'Data migration failed: old table has % rows, new table has % rows', old_count, new_count;
    END IF;
    
    RAISE NOTICE 'Data migration successful: % rows migrated', new_count;
END $$;

-- Step 8: Drop old table (comment out if you want to keep it for safety)
-- DROP TABLE analytics_impressions_old;

-- Note: Keep old table for now, can be dropped after verification
-- To rollback: DROP TABLE analytics_impressions; ALTER TABLE analytics_impressions_old RENAME TO analytics_impressions;
