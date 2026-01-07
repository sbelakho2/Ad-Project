-- Rollback: Remove review queue columns from recon_match tables

-- Drop indexes first
DROP INDEX IF EXISTS idx_recon_match_publisher_id;
DROP INDEX IF EXISTS idx_recon_match_review_status;
DROP INDEX IF EXISTS idx_recon_match_confidence_review;
DROP INDEX IF EXISTS idx_recon_match_review_publisher_id;
DROP INDEX IF EXISTS idx_recon_match_review_status;

-- Remove columns from recon_match
ALTER TABLE recon_match 
DROP COLUMN IF EXISTS publisher_id,
DROP COLUMN IF EXISTS expected_id,
DROP COLUMN IF EXISTS match_confidence,
DROP COLUMN IF EXISTS match_type,
DROP COLUMN IF EXISTS discrepancy_usd,
DROP COLUMN IF EXISTS review_status,
DROP COLUMN IF EXISTS reviewed_at,
DROP COLUMN IF EXISTS reviewed_by,
DROP COLUMN IF EXISTS review_notes,
DROP COLUMN IF EXISTS created_at;

-- Remove columns from recon_match_review
ALTER TABLE recon_match_review 
DROP COLUMN IF EXISTS publisher_id,
DROP COLUMN IF EXISTS match_confidence,
DROP COLUMN IF EXISTS match_type,
DROP COLUMN IF EXISTS discrepancy_usd,
DROP COLUMN IF EXISTS review_status,
DROP COLUMN IF EXISTS reviewed_at,
DROP COLUMN IF EXISTS reviewed_by,
DROP COLUMN IF EXISTS review_notes;
