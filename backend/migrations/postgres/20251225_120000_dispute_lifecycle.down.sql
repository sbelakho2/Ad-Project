-- Rollback: Remove dispute lifecycle columns

-- Drop check constraint
ALTER TABLE recon_disputes DROP CONSTRAINT IF EXISTS recon_disputes_status_check;

-- Drop indexes
DROP INDEX IF EXISTS idx_recon_disputes_publisher_id;
DROP INDEX IF EXISTS idx_recon_disputes_status;
DROP INDEX IF EXISTS idx_recon_disputes_network_ref;

-- Remove columns
ALTER TABLE recon_disputes 
DROP COLUMN IF EXISTS publisher_id,
DROP COLUMN IF EXISTS contact,
DROP COLUMN IF EXISTS notes,
DROP COLUMN IF EXISTS network_ref_id,
DROP COLUMN IF EXISTS resolution_amount,
DROP COLUMN IF EXISTS resolution_notes,
DROP COLUMN IF EXISTS submitted_at,
DROP COLUMN IF EXISTS resolved_at;
