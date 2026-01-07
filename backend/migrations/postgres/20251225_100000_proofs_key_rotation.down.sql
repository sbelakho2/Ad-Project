-- Rollback: Remove key_id columns from proofs tables

-- Drop indexes first
DROP INDEX IF EXISTS idx_proofs_daily_roots_key_id;
DROP INDEX IF EXISTS idx_proofs_monthly_digest_key_id;

-- Drop columns
ALTER TABLE proofs_daily_roots DROP COLUMN IF EXISTS key_id;
ALTER TABLE proofs_monthly_digest DROP COLUMN IF EXISTS key_id;
