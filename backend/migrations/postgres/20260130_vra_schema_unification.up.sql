-- VRA Schema Unification Migration
-- Resolves conflicts between 20251130_vra_postgres_schema and 20251212_vra_network_connections
-- Uses ALTER TABLE to safely add missing columns without dropping existing data

SET LOCAL lock_timeout = '10s';
SET LOCAL statement_timeout = '120s';

-- ============================================================================
-- STEP 1: Add missing multi-tenancy columns to core VRA tables
-- ============================================================================

-- Add publisher_id to recon_statements_norm (if not exists from 20251212 migration)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_statements_norm' 
                   AND column_name = 'publisher_id') THEN
        ALTER TABLE recon_statements_norm ADD COLUMN publisher_id UUID;
        CREATE INDEX IF NOT EXISTS idx_recon_statements_norm_publisher 
            ON recon_statements_norm(publisher_id, event_date);
        RAISE NOTICE 'Added publisher_id to recon_statements_norm';
    END IF;
END $$;

-- Add publisher_id to recon_expected (if not exists)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_expected' 
                   AND column_name = 'publisher_id') THEN
        ALTER TABLE recon_expected ADD COLUMN publisher_id UUID;
        CREATE INDEX IF NOT EXISTS idx_recon_expected_publisher 
            ON recon_expected(publisher_id, ts);
        RAISE NOTICE 'Added publisher_id to recon_expected';
    END IF;
END $$;

-- Add publisher_id to recon_match (if not exists)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'publisher_id') THEN
        ALTER TABLE recon_match ADD COLUMN publisher_id UUID;
        CREATE INDEX IF NOT EXISTS idx_recon_match_publisher 
            ON recon_match(publisher_id, matched_at DESC);
        RAISE NOTICE 'Added publisher_id to recon_match';
    END IF;
END $$;

-- Add publisher_id to recon_match_review (if not exists)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match_review' 
                   AND column_name = 'publisher_id') THEN
        ALTER TABLE recon_match_review ADD COLUMN publisher_id UUID;
        CREATE INDEX IF NOT EXISTS idx_recon_match_review_publisher 
            ON recon_match_review(publisher_id, matched_at DESC);
        RAISE NOTICE 'Added publisher_id to recon_match_review';
    END IF;
END $$;

-- Add publisher_id to recon_deltas (ensured by 20251212 but add guard)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_deltas' 
                   AND column_name = 'publisher_id') THEN
        ALTER TABLE recon_deltas ADD COLUMN publisher_id UUID;
        CREATE INDEX IF NOT EXISTS idx_recon_deltas_publisher 
            ON recon_deltas(publisher_id, window_start);
        RAISE NOTICE 'Added publisher_id to recon_deltas';
    END IF;
END $$;

-- Add publisher_id to recon_disputes (if not exists)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'publisher_id') THEN
        ALTER TABLE recon_disputes ADD COLUMN publisher_id UUID;
        CREATE INDEX IF NOT EXISTS idx_recon_disputes_publisher 
            ON recon_disputes(publisher_id, created_at DESC);
        RAISE NOTICE 'Added publisher_id to recon_disputes';
    END IF;
END $$;

-- ============================================================================
-- STEP 2: Add missing columns needed for getReviewQueue join
-- ============================================================================

-- Add match_confidence column (named consistency with controller expectations)
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'match_confidence') THEN
        -- If link_confidence exists, create match_confidence as alias-compatible
        IF EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'link_confidence') THEN
            ALTER TABLE recon_match ADD COLUMN match_confidence NUMERIC(5,2);
            -- Copy existing data
            UPDATE recon_match SET match_confidence = link_confidence WHERE match_confidence IS NULL;
            RAISE NOTICE 'Added match_confidence (copied from link_confidence) to recon_match';
        ELSE
            ALTER TABLE recon_match ADD COLUMN match_confidence NUMERIC(5,2) DEFAULT 0;
            RAISE NOTICE 'Added match_confidence to recon_match';
        END IF;
    END IF;
END $$;

-- Add match_type column for fuzzy/exact classification
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'match_type') THEN
        ALTER TABLE recon_match ADD COLUMN match_type TEXT DEFAULT 'fuzzy';
        RAISE NOTICE 'Added match_type to recon_match';
    END IF;
END $$;

-- Add discrepancy_usd column for variance tracking
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'discrepancy_usd') THEN
        ALTER TABLE recon_match ADD COLUMN discrepancy_usd NUMERIC(18,6) DEFAULT 0;
        RAISE NOTICE 'Added discrepancy_usd to recon_match';
    END IF;
END $$;

-- Add review_status column for review queue tracking
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'review_status') THEN
        ALTER TABLE recon_match ADD COLUMN review_status TEXT; -- NULL = not reviewed, 'approved', 'rejected'
        CREATE INDEX IF NOT EXISTS idx_recon_match_review_status 
            ON recon_match(review_status) WHERE review_status IS NULL;
        RAISE NOTICE 'Added review_status to recon_match';
    END IF;
END $$;

-- Add expected_id column for joining to recon_expected
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_match' 
                   AND column_name = 'expected_id') THEN
        ALTER TABLE recon_match ADD COLUMN expected_id BIGINT;
        CREATE INDEX IF NOT EXISTS idx_recon_match_expected 
            ON recon_match(expected_id);
        RAISE NOTICE 'Added expected_id to recon_match';
    END IF;
END $$;

-- ============================================================================
-- STEP 3: Add functional index for JSONB request_id lookups on analytics_revenue_events
-- This is critical for performance - prevents full table scans on the largest table
-- ============================================================================

-- Create functional index on analytics_revenue_events for request_id lookups
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables 
               WHERE table_name = 'analytics_revenue_events') THEN
        CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_analytics_revenue_events_request_id
            ON analytics_revenue_events ((metadata->>'request_id'))
            WHERE metadata->>'request_id' IS NOT NULL;
        RAISE NOTICE 'Created functional index on analytics_revenue_events(metadata->>request_id)';
    END IF;
EXCEPTION WHEN others THEN
    -- CONCURRENTLY cannot be used in a transaction, so handle gracefully
    RAISE NOTICE 'Could not create index concurrently; may need to run manually: CREATE INDEX CONCURRENTLY idx_analytics_revenue_events_request_id ON analytics_revenue_events ((metadata->>''request_id'')) WHERE metadata->>''request_id'' IS NOT NULL';
END $$;

-- Add composite index for publisher + request_id lookups
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables 
               WHERE table_name = 'analytics_revenue_events') THEN
        CREATE INDEX IF NOT EXISTS idx_analytics_revenue_events_publisher_request
            ON analytics_revenue_events (publisher_id, (metadata->>'request_id'))
            WHERE metadata->>'request_id' IS NOT NULL;
        RAISE NOTICE 'Created publisher + request_id composite index on analytics_revenue_events';
    END IF;
EXCEPTION WHEN others THEN
    RAISE NOTICE 'Could not create composite index: %', SQLERRM;
END $$;

-- ============================================================================
-- STEP 4: Add network_type to recon_statements_norm for controller compatibility
-- ============================================================================

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_statements_norm' 
                   AND column_name = 'network_type') THEN
        -- If 'network' column exists, create alias
        IF EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_statements_norm' 
                   AND column_name = 'network') THEN
            ALTER TABLE recon_statements_norm ADD COLUMN network_type TEXT;
            UPDATE recon_statements_norm SET network_type = network WHERE network_type IS NULL;
            RAISE NOTICE 'Added network_type (copied from network) to recon_statements_norm';
        ELSE
            ALTER TABLE recon_statements_norm ADD COLUMN network_type TEXT DEFAULT '';
            RAISE NOTICE 'Added network_type to recon_statements_norm';
        END IF;
    END IF;
END $$;

-- ============================================================================
-- STEP 5: Ensure recon_disputes has all columns for dispute lifecycle
-- ============================================================================

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'contact') THEN
        ALTER TABLE recon_disputes ADD COLUMN contact TEXT;
        RAISE NOTICE 'Added contact to recon_disputes';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'notes') THEN
        ALTER TABLE recon_disputes ADD COLUMN notes TEXT;
        RAISE NOTICE 'Added notes to recon_disputes';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'network_ref_id') THEN
        ALTER TABLE recon_disputes ADD COLUMN network_ref_id TEXT;
        RAISE NOTICE 'Added network_ref_id to recon_disputes';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'resolution_amount') THEN
        ALTER TABLE recon_disputes ADD COLUMN resolution_amount NUMERIC(18,6);
        RAISE NOTICE 'Added resolution_amount to recon_disputes';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'resolution_notes') THEN
        ALTER TABLE recon_disputes ADD COLUMN resolution_notes TEXT;
        RAISE NOTICE 'Added resolution_notes to recon_disputes';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'submitted_at') THEN
        ALTER TABLE recon_disputes ADD COLUMN submitted_at TIMESTAMPTZ;
        RAISE NOTICE 'Added submitted_at to recon_disputes';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                   WHERE table_name = 'recon_disputes' 
                   AND column_name = 'resolved_at') THEN
        ALTER TABLE recon_disputes ADD COLUMN resolved_at TIMESTAMPTZ;
        RAISE NOTICE 'Added resolved_at to recon_disputes';
    END IF;
END $$;

-- ============================================================================
-- COMPLETE
-- ============================================================================
