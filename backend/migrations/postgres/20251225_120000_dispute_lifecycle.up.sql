-- Migration: Add dispute lifecycle columns to recon_disputes table
-- Part of VRA P1 fix for complete dispute lifecycle management

-- Add columns for full lifecycle tracking
ALTER TABLE recon_disputes 
ADD COLUMN IF NOT EXISTS publisher_id uuid,
ADD COLUMN IF NOT EXISTS contact text,
ADD COLUMN IF NOT EXISTS notes text,
ADD COLUMN IF NOT EXISTS network_ref_id text,           -- External reference from network
ADD COLUMN IF NOT EXISTS resolution_amount numeric(18,6),  -- Final resolved amount
ADD COLUMN IF NOT EXISTS resolution_notes text,         -- Network's resolution notes
ADD COLUMN IF NOT EXISTS submitted_at timestamptz,      -- When submitted to network
ADD COLUMN IF NOT EXISTS resolved_at timestamptz;       -- When finally resolved

-- Add index for publisher lookups
CREATE INDEX IF NOT EXISTS idx_recon_disputes_publisher_id ON recon_disputes (publisher_id);

-- Add index for status queries
CREATE INDEX IF NOT EXISTS idx_recon_disputes_status ON recon_disputes (status);

-- Add index for network reference lookups  
CREATE INDEX IF NOT EXISTS idx_recon_disputes_network_ref ON recon_disputes (network_ref_id) WHERE network_ref_id IS NOT NULL;

-- Add check constraint for valid status values
DO $$ 
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'recon_disputes_status_check'
  ) THEN
    ALTER TABLE recon_disputes 
    ADD CONSTRAINT recon_disputes_status_check 
    CHECK (status IN ('draft', 'submitted', 'in_progress', 'resolved', 'rejected', 'cancelled'));
  END IF;
END $$;

COMMENT ON COLUMN recon_disputes.status IS 'Lifecycle status: draft, submitted, in_progress, resolved, rejected, cancelled';
COMMENT ON COLUMN recon_disputes.network_ref_id IS 'External reference ID from the ad network';
COMMENT ON COLUMN recon_disputes.resolution_amount IS 'Final resolved amount (may differ from disputed amount)';
