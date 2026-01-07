-- Migration: 039_webhook_retry_tracking.sql
-- Description: Add retry tracking columns to stripe_webhook_events table

-- Add columns for retry tracking
ALTER TABLE stripe_webhook_events 
  ADD COLUMN IF NOT EXISTS error_message TEXT,
  ADD COLUMN IF NOT EXISTS retry_count INTEGER DEFAULT 0,
  ADD COLUMN IF NOT EXISTS last_error_at TIMESTAMP WITH TIME ZONE;

-- Add index for failed events that need retry
CREATE INDEX IF NOT EXISTS idx_stripe_webhook_events_retry 
  ON stripe_webhook_events(retry_count) 
  WHERE processed = false AND retry_count > 0;

-- Add columns for dunning_attempts table to support subscription_id lookup
ALTER TABLE dunning_attempts 
  ADD COLUMN IF NOT EXISTS subscription_id VARCHAR(255);

-- Create index on subscription_id for dunning lookups
CREATE INDEX IF NOT EXISTS idx_dunning_attempts_subscription 
  ON dunning_attempts(subscription_id) 
  WHERE status IN ('pending', 'retrying');

-- Add status 'cancelled' to dunning_attempts if not exists
DO $$
BEGIN
  -- Check if the constraint exists and update it
  IF EXISTS (
    SELECT 1 FROM pg_constraint 
    WHERE conname = 'dunning_attempts_status_check' 
    AND conrelid = 'dunning_attempts'::regclass
  ) THEN
    ALTER TABLE dunning_attempts DROP CONSTRAINT IF EXISTS dunning_attempts_status_check;
  END IF;
  
  ALTER TABLE dunning_attempts 
    ADD CONSTRAINT dunning_attempts_status_check 
    CHECK (status IN ('pending', 'retrying', 'succeeded', 'failed', 'suspended', 'cancelled'));
EXCEPTION
  WHEN others THEN
    -- Constraint may not exist, that's fine
    NULL;
END
$$;

-- Add updated_at column to dunning_attempts if not exists
ALTER TABLE dunning_attempts 
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP;

COMMENT ON COLUMN stripe_webhook_events.error_message IS 'Error message from last failed processing attempt';
COMMENT ON COLUMN stripe_webhook_events.retry_count IS 'Number of processing retry attempts';
COMMENT ON COLUMN stripe_webhook_events.last_error_at IS 'Timestamp of last processing error';
COMMENT ON COLUMN dunning_attempts.subscription_id IS 'Stripe subscription ID for lookup from webhooks';
