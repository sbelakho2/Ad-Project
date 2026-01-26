-- ============================================================================
-- Migration: Add lift percentage fields to sa_contacts
-- Date: January 2026
-- Purpose: Store calculated lift estimates for "Truthful Puffery" in emails
-- ============================================================================

BEGIN;

-- Add lift estimate columns to sa_contacts
ALTER TABLE sa_contacts 
  ADD COLUMN IF NOT EXISTS lift_percent INTEGER,
  ADD COLUMN IF NOT EXISTS lift_range VARCHAR(20),
  ADD COLUMN IF NOT EXISTS lift_confidence VARCHAR(10),
  ADD COLUMN IF NOT EXISTS lift_reasoning TEXT,
  ADD COLUMN IF NOT EXISTS current_ad_setup VARCHAR(100);

-- Add index for filtering by lift potential
CREATE INDEX IF NOT EXISTS idx_sa_contacts_lift_percent 
  ON sa_contacts (lift_percent DESC) 
  WHERE lift_percent IS NOT NULL;

-- Add comment explaining purpose
COMMENT ON COLUMN sa_contacts.lift_percent IS 'Estimated revenue lift percentage based on current ad setup analysis';
COMMENT ON COLUMN sa_contacts.lift_range IS 'Human-readable range like "15-22%"';
COMMENT ON COLUMN sa_contacts.lift_confidence IS 'Confidence level: high/medium/low';
COMMENT ON COLUMN sa_contacts.lift_reasoning IS 'Detailed explanation for the estimate (used in personalization)';
COMMENT ON COLUMN sa_contacts.current_ad_setup IS 'Description of current monetization setup';

COMMIT;
