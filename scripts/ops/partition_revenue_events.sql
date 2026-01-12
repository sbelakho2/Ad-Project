-- Partition analytics_revenue_events by day
-- This script converts the existing table to a partitioned table
--
-- Run this during a maintenance window as it requires table recreation

BEGIN;

-- Step 1: Rename existing table
ALTER TABLE public.analytics_revenue_events RENAME TO analytics_revenue_events_old;

-- Step 2: Create new partitioned table with same structure
CREATE TABLE public.analytics_revenue_events (
    id BIGSERIAL,
    event_id UUID NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    publisher_id UUID NOT NULL,
    app_id UUID,
    placement_id UUID,
    adapter_id TEXT,
    adapter_name TEXT,
    impression_id UUID,
    revenue_type TEXT,
    revenue_usd NUMERIC(12, 6),
    revenue_currency CHAR(3),
    revenue_original NUMERIC(12, 6),
    exchange_rate NUMERIC(12, 6),
    ecpm_usd NUMERIC(12, 6),
    country_code CHAR(2),
    ad_format TEXT,
    os TEXT,
    is_test_mode BOOLEAN DEFAULT FALSE,
    metadata JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
) PARTITION BY RANGE (observed_at);

-- Step 3: Create partitions for existing data range and future dates
-- Past partitions (adjust based on your data range)
CREATE TABLE public.analytics_revenue_events_2025_12_29 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2025-12-29') TO ('2025-12-30');
CREATE TABLE public.analytics_revenue_events_2025_12_30 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2025-12-30') TO ('2025-12-31');
CREATE TABLE public.analytics_revenue_events_2025_12_31 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2025-12-31') TO ('2026-01-01');

-- January 2026 partitions
CREATE TABLE public.analytics_revenue_events_2026_01_01 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-01') TO ('2026-01-02');
CREATE TABLE public.analytics_revenue_events_2026_01_02 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-02') TO ('2026-01-03');
CREATE TABLE public.analytics_revenue_events_2026_01_03 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-03') TO ('2026-01-04');
CREATE TABLE public.analytics_revenue_events_2026_01_04 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-04') TO ('2026-01-05');
CREATE TABLE public.analytics_revenue_events_2026_01_05 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-05') TO ('2026-01-06');
CREATE TABLE public.analytics_revenue_events_2026_01_06 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-06') TO ('2026-01-07');
CREATE TABLE public.analytics_revenue_events_2026_01_07 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-07') TO ('2026-01-08');
CREATE TABLE public.analytics_revenue_events_2026_01_08 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-08') TO ('2026-01-09');
CREATE TABLE public.analytics_revenue_events_2026_01_09 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-09') TO ('2026-01-10');
CREATE TABLE public.analytics_revenue_events_2026_01_10 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-10') TO ('2026-01-11');
CREATE TABLE public.analytics_revenue_events_2026_01_11 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-11') TO ('2026-01-12');
CREATE TABLE public.analytics_revenue_events_2026_01_12 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-12') TO ('2026-01-13');
CREATE TABLE public.analytics_revenue_events_2026_01_13 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-13') TO ('2026-01-14');
CREATE TABLE public.analytics_revenue_events_2026_01_14 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-14') TO ('2026-01-15');
CREATE TABLE public.analytics_revenue_events_2026_01_15 PARTITION OF public.analytics_revenue_events
    FOR VALUES FROM ('2026-01-15') TO ('2026-01-16');

-- Step 4: Create indexes on partitioned table
CREATE INDEX idx_revenue_events_observed_at ON public.analytics_revenue_events (observed_at);
CREATE INDEX idx_revenue_events_publisher_id ON public.analytics_revenue_events (publisher_id);
CREATE INDEX idx_revenue_events_impression_id ON public.analytics_revenue_events (impression_id);
CREATE UNIQUE INDEX idx_revenue_events_event_unique ON public.analytics_revenue_events (event_id, observed_at);

-- Step 5: Copy data from old table to new partitioned table
INSERT INTO public.analytics_revenue_events (
    event_id, observed_at, publisher_id, app_id, placement_id,
    adapter_id, adapter_name, impression_id, revenue_type,
    revenue_usd, revenue_currency, revenue_original, exchange_rate,
    ecpm_usd, country_code, ad_format, os, is_test_mode, metadata, created_at
)
SELECT 
    event_id, observed_at, publisher_id, app_id, placement_id,
    adapter_id, adapter_name, impression_id, revenue_type,
    revenue_usd, revenue_currency, revenue_original, exchange_rate,
    ecpm_usd, country_code, ad_format, os, is_test_mode, metadata, 
    COALESCE(created_at, observed_at)
FROM public.analytics_revenue_events_old;

-- Step 6: Verify row counts match
DO $$
DECLARE
    old_count BIGINT;
    new_count BIGINT;
BEGIN
    SELECT count(*) INTO old_count FROM public.analytics_revenue_events_old;
    SELECT count(*) INTO new_count FROM public.analytics_revenue_events;
    
    IF old_count != new_count THEN
        RAISE EXCEPTION 'Row count mismatch! Old: %, New: %', old_count, new_count;
    END IF;
    
    RAISE NOTICE 'Migration successful: % rows migrated', new_count;
END $$;

COMMIT;

-- After successful migration, drop the old table (run separately after verification)
-- DROP TABLE public.analytics_revenue_events_old;
