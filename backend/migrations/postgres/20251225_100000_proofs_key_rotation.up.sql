-- Migration: Add key_id column to proofs tables for key rotation support
-- Part of VRA P0 security fix

-- Add key_id column to proofs_daily_roots for tracking which key signed each root
ALTER TABLE proofs_daily_roots 
ADD COLUMN IF NOT EXISTS key_id text NOT NULL DEFAULT '';

-- Add key_id column to proofs_monthly_digest for tracking which key signed each digest
ALTER TABLE proofs_monthly_digest 
ADD COLUMN IF NOT EXISTS key_id text NOT NULL DEFAULT '';

-- Create index on key_id for efficient lookups during verification
CREATE INDEX IF NOT EXISTS idx_proofs_daily_roots_key_id ON proofs_daily_roots (key_id);
CREATE INDEX IF NOT EXISTS idx_proofs_monthly_digest_key_id ON proofs_monthly_digest (key_id);

COMMENT ON COLUMN proofs_daily_roots.key_id IS 'ID of the signing key used (e.g., v1, 2024-01). Enables key rotation with grace period.';
COMMENT ON COLUMN proofs_monthly_digest.key_id IS 'ID of the signing key used (e.g., v1, 2024-01). Enables key rotation with grace period.';
