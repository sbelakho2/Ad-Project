-- Migration: Add review queue columns to recon_match table
-- Part of VRA P1 fix for review queue functionality

-- Add columns for review status and tracking
ALTER TABLE recon_match 
ADD COLUMN IF NOT EXISTS publisher_id uuid,
ADD COLUMN IF NOT EXISTS expected_id bigint,
ADD COLUMN IF NOT EXISTS match_confidence numeric(5,4) DEFAULT 0,
ADD COLUMN IF NOT EXISTS match_type text DEFAULT 'exact',
ADD COLUMN IF NOT EXISTS discrepancy_usd numeric(18,6) DEFAULT 0,
ADD COLUMN IF NOT EXISTS review_status text,
ADD COLUMN IF NOT EXISTS reviewed_at timestamptz,
ADD COLUMN IF NOT EXISTS reviewed_by text,
ADD COLUMN IF NOT EXISTS review_notes text,
ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();

-- Create indexes for efficient review queue queries
CREATE INDEX IF NOT EXISTS idx_recon_match_publisher_id ON recon_match (publisher_id);
CREATE INDEX IF NOT EXISTS idx_recon_match_review_status ON recon_match (review_status) WHERE review_status IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recon_match_confidence_review ON recon_match (match_confidence, review_status) 
  WHERE match_confidence >= 0.4 AND match_confidence <= 0.8;

-- Add constraints
COMMENT ON COLUMN recon_match.review_status IS 'Review status: NULL (pending), approved, rejected';
COMMENT ON COLUMN recon_match.match_confidence IS 'Match confidence 0-1. Values 0.4-0.8 go to review queue.';
COMMENT ON COLUMN recon_match.match_type IS 'Match type: exact, fuzzy_amount, fuzzy_timing, fuzzy_placement';

-- Also update recon_match_review for consistency
ALTER TABLE recon_match_review 
ADD COLUMN IF NOT EXISTS publisher_id uuid,
ADD COLUMN IF NOT EXISTS match_confidence numeric(5,4) DEFAULT 0,
ADD COLUMN IF NOT EXISTS match_type text DEFAULT 'fuzzy',
ADD COLUMN IF NOT EXISTS discrepancy_usd numeric(18,6) DEFAULT 0,
ADD COLUMN IF NOT EXISTS review_status text DEFAULT 'pending',
ADD COLUMN IF NOT EXISTS reviewed_at timestamptz,
ADD COLUMN IF NOT EXISTS reviewed_by text,
ADD COLUMN IF NOT EXISTS review_notes text;

CREATE INDEX IF NOT EXISTS idx_recon_match_review_publisher_id ON recon_match_review (publisher_id);
CREATE INDEX IF NOT EXISTS idx_recon_match_review_status ON recon_match_review (review_status);
