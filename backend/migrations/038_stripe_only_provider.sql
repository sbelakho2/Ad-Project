-- Migration: 038_stripe_only_provider.sql
-- Description: Update payment provider to Stripe only, removing Tipalti/Wise/Payoneer
-- Date: 2025-01-XX

-- Update the payout_ledger provider constraint to only allow 'stripe'
-- Note: We need to handle existing data by converting old provider values to 'stripe'

-- First, update any existing records with old provider names
UPDATE payout_ledger 
SET provider = 'stripe' 
WHERE provider IN ('tipalti', 'wise', 'payoneer');

-- Drop the old constraint if it exists
DO $$ 
BEGIN
    ALTER TABLE payout_ledger DROP CONSTRAINT IF EXISTS payout_ledger_provider_check;
EXCEPTION
    WHEN undefined_object THEN NULL;
END $$;

-- Add new constraint allowing only stripe
ALTER TABLE payout_ledger 
ADD CONSTRAINT payout_ledger_provider_check 
CHECK (provider = 'stripe');

-- Update comment
COMMENT ON COLUMN payout_ledger.provider IS 'Payment provider used: stripe only';

-- Update payouts table provider column as well
UPDATE payouts 
SET provider = 'stripe' 
WHERE provider IN ('tipalti', 'wise', 'payoneer');

-- Add constraint to payouts table if it doesn't have one
DO $$ 
BEGIN
    ALTER TABLE payouts DROP CONSTRAINT IF EXISTS payouts_provider_check;
EXCEPTION
    WHEN undefined_object THEN NULL;
END $$;

-- Note: We don't add a strict constraint on payouts.provider as it may contain historical data
-- or NULL values for payouts not yet processed

-- Log the migration
DO $$
BEGIN
    RAISE NOTICE 'Migration 038: Successfully updated payment providers to Stripe only';
END $$;
