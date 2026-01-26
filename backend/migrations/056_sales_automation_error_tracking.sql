-- Migration 056: Add error tracking to sales automation enrollments
-- Created: January 2026
-- Purpose: Enable template lint to record errors and prevent broken emails
-- NOTE: This migration only applies if V1 sales automation tables exist

-- Add error_message column to track template lint failures
DO $$
BEGIN
  -- Only run if the table exists
  IF EXISTS (
    SELECT 1 FROM information_schema.tables 
    WHERE table_name = 'outreach_enrollments'
  ) THEN
    IF NOT EXISTS (
      SELECT 1 FROM information_schema.columns 
      WHERE table_name = 'outreach_enrollments' AND column_name = 'error_message'
    ) THEN
      ALTER TABLE outreach_enrollments ADD COLUMN error_message TEXT;
      COMMENT ON COLUMN outreach_enrollments.error_message IS 'Stores error details when enrollment fails (e.g., unresolved placeholders)';
    END IF;
  END IF;
END
$$;

-- Add 'error' as valid status (used when template lint fails)
-- Check constraint update: add 'error' to allowed statuses if constraint exists
DO $$
BEGIN
  -- Only run if the table exists
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.tables 
    WHERE table_name = 'outreach_enrollments'
  ) THEN
    RETURN;
  END IF;

  -- Drop old constraint if it exists
  IF EXISTS (
    SELECT 1 FROM information_schema.constraint_column_usage
    WHERE table_name = 'outreach_enrollments' AND constraint_name LIKE '%status%check%'
  ) THEN
    ALTER TABLE outreach_enrollments DROP CONSTRAINT IF EXISTS outreach_enrollments_status_check;
  END IF;
  
  -- Add new constraint with 'error' status
  ALTER TABLE outreach_enrollments 
    ADD CONSTRAINT outreach_enrollments_status_check 
    CHECK (status IN ('active', 'completed', 'paused', 'responded', 'converted', 'bounced', 'opted_out', 'error'));
EXCEPTION
  WHEN duplicate_object THEN NULL;
  WHEN others THEN NULL;
END
$$;

-- Index for finding errored enrollments (useful for admin monitoring)
-- Only create if table exists
DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.tables 
    WHERE table_name = 'outreach_enrollments'
  ) THEN
    CREATE INDEX IF NOT EXISTS idx_enrollments_error 
      ON outreach_enrollments(status, created_at) 
      WHERE status = 'error';
  END IF;
END
$$;

-- Add comment documenting the lint process (only if table exists)
DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.tables 
    WHERE table_name = 'outreach_enrollments'
  ) THEN
    COMMENT ON TABLE outreach_enrollments IS 
      'Contacts enrolled in outreach sequences. Status=error indicates template lint failure (unresolved placeholders).';
  END IF;
END
$$;
