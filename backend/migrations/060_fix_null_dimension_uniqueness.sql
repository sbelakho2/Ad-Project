-- Migration 060: Fix NULL Dimension Uniqueness Vulnerability
-- 
-- CRITICAL DATA INTEGRITY FIX
-- 
-- Problem: PostgreSQL's UNIQUE constraint treats NULL != NULL, meaning multiple rows
-- with identical keys but NULL dimensions won't trigger conflicts. This causes:
-- 1. Duplicate revenue entries when networks report data with missing dimensions
-- 2. Revenue over-counting (e.g., $100 becomes $200 after re-sync)
-- 3. Untrustworthy variance calculations
--
-- Example scenario:
--   Sync Run 1: INSERT (publisher='X', network='admob', date='2026-01-15', app_id=NULL, ad_unit_id='xyz')
--   Sync Run 2: INSERT same data - NO CONFLICT because NULL != NULL in the unique constraint
--   Result: Revenue is doubled
--
-- Solution: Replace the simple UNIQUE constraint with a functional unique index using COALESCE
-- to treat NULL as empty string for uniqueness purposes
--
-- Affected networks:
--   - AdMob: often missing app_id
--   - Unity: often missing ad_unit_id
--   - AppLovin: may have NULL country
--   - IronSource: variable NULL patterns

BEGIN;

-- Step 1: Drop the existing unique constraint (if exists)
-- The constraint name may vary - try both patterns
DO $$
BEGIN
    -- Try dropping as a constraint
    BEGIN
        ALTER TABLE recon_statements_norm 
        DROP CONSTRAINT IF EXISTS recon_statements_norm_publisher_id_network_type_event_date_a_key;
    EXCEPTION WHEN undefined_object THEN
        NULL; -- Ignore if not found
    END;
    
    -- Try alternative constraint name
    BEGIN
        ALTER TABLE recon_statements_norm 
        DROP CONSTRAINT IF EXISTS recon_statements_norm_unique;
    EXCEPTION WHEN undefined_object THEN
        NULL;
    END;
    
    -- Also drop any existing functional index (idempotency)
    DROP INDEX IF EXISTS idx_recon_statements_norm_unique_coalesced;
END $$;

-- Step 2: Create the new functional unique index
-- Uses COALESCE to convert NULLs to empty strings for uniqueness comparison
-- This ensures (publisher, network, date, NULL, 'xyz', 'US', 'banner') will conflict
-- with (publisher, network, date, NULL, 'xyz', 'US', 'banner') on re-insert
CREATE UNIQUE INDEX idx_recon_statements_norm_unique_coalesced
ON recon_statements_norm (
    publisher_id,
    network_type,
    event_date,
    COALESCE(app_id, ''),
    COALESCE(ad_unit_id, ''),
    COALESCE(country, ''),
    COALESCE(format, '')
);

-- Step 3: Add a trigger to detect and log potential duplicates before the index catches them
-- This helps with debugging and monitoring
CREATE OR REPLACE FUNCTION check_recon_statement_duplicate()
RETURNS TRIGGER AS $$
DECLARE
    existing_count INTEGER;
BEGIN
    -- Check for pre-existing row with same coalesced key
    SELECT COUNT(*) INTO existing_count
    FROM recon_statements_norm
    WHERE publisher_id = NEW.publisher_id
      AND network_type = NEW.network_type
      AND event_date = NEW.event_date
      AND COALESCE(app_id, '') = COALESCE(NEW.app_id, '')
      AND COALESCE(ad_unit_id, '') = COALESCE(NEW.ad_unit_id, '')
      AND COALESCE(country, '') = COALESCE(NEW.country, '')
      AND COALESCE(format, '') = COALESCE(NEW.format, '');
    
    -- If a duplicate would be created, raise a notice (the unique index will block the insert)
    IF existing_count > 0 THEN
        RAISE NOTICE 'Duplicate statement detected for publisher=%, network=%, date=%, app=%, unit=%, country=%, format=%',
            NEW.publisher_id, NEW.network_type, NEW.event_date, 
            COALESCE(NEW.app_id, 'NULL'), COALESCE(NEW.ad_unit_id, 'NULL'),
            COALESCE(NEW.country, 'NULL'), COALESCE(NEW.format, 'NULL');
    END IF;
    
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_recon_statement_duplicate_check ON recon_statements_norm;
CREATE TRIGGER trg_recon_statement_duplicate_check
    BEFORE INSERT ON recon_statements_norm
    FOR EACH ROW
    EXECUTE FUNCTION check_recon_statement_duplicate();

-- Step 4: Clean up any existing duplicates (caused by the bug)
-- First, identify and log them
DO $$
DECLARE
    dup_count INTEGER;
BEGIN
    SELECT COUNT(*) INTO dup_count
    FROM (
        SELECT publisher_id, network_type, event_date,
               COALESCE(app_id, '') as app_key,
               COALESCE(ad_unit_id, '') as unit_key,
               COALESCE(country, '') as country_key,
               COALESCE(format, '') as format_key
        FROM recon_statements_norm
        GROUP BY publisher_id, network_type, event_date,
                 COALESCE(app_id, ''), COALESCE(ad_unit_id, ''),
                 COALESCE(country, ''), COALESCE(format, '')
        HAVING COUNT(*) > 1
    ) dups;
    
    IF dup_count > 0 THEN
        RAISE NOTICE 'Found % duplicate statement groups to clean up', dup_count;
    END IF;
END $$;

-- Delete duplicates, keeping the most recent one (by created_at)
-- This uses a CTE to identify duplicates and delete all but the newest
WITH duplicates AS (
    SELECT id,
           ROW_NUMBER() OVER (
               PARTITION BY publisher_id, network_type, event_date,
                           COALESCE(app_id, ''), COALESCE(ad_unit_id, ''),
                           COALESCE(country, ''), COALESCE(format, '')
               ORDER BY created_at DESC, id DESC
           ) AS rn
    FROM recon_statements_norm
)
DELETE FROM recon_statements_norm
WHERE id IN (SELECT id FROM duplicates WHERE rn > 1);

-- Step 5: Add comment documenting the constraint
COMMENT ON INDEX idx_recon_statements_norm_unique_coalesced IS 
'Functional unique index using COALESCE to handle NULL dimensions. 
Networks often report partial dimensions (e.g., Unity without ad_unit_id, AdMob without app_id).
Without COALESCE, PostgreSQL treats NULL != NULL, causing duplicate entries.
Created by migration 060 to fix CVE-like revenue duplication vulnerability.';

COMMIT;
