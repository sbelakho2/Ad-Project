-- ============================================================================
-- Migration 052: Seed Autonomous Sales Data
-- ============================================================================
-- Seeds Thompson Sampler arms and ensures core data exists for the
-- autonomous sales system to function properly.
--
-- This migration:
--   1. Seeds subject_line bandit arms for A/B testing
--   2. Ensures sa_spintax_templates has the initial_outreach template type
--   3. Fixes any schema issues from prior migrations
-- ============================================================================

-- ============================================================================
-- 1. SEED THOMPSON SAMPLER ARMS (Required for /compose endpoint)
-- ============================================================================
-- These subject line variations will be A/B tested via Thompson Sampling.
-- The bandit learns which subject lines get the most opens/replies.
-- IMPROVED (Jan 2026): Replaced generic "quick question" and "Hi from" patterns
-- with more specific, benefit-oriented subject lines that lead with value.

INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, total_trials, total_successes, active)
VALUES 
  -- Subject line variations for cold outreach - benefit-first approach
  ('subject_line', 'monetization_question', '{{APP_NAME}} monetization question', 1, 1, 0, 0, true),
  ('subject_line', 'noticed_store', 'Noticed {{APP_NAME}} in the store', 1, 1, 0, 0, true),
  ('subject_line', 'ad_revenue_idea', 'Idea for {{APP_NAME}} ad revenue', 1, 1, 0, 0, true),
  ('subject_line', 'publisher_connect', '{{COMPANY}} + ApexMediation?', 1, 1, 0, 0, true),
  ('subject_line', 'mediation_fit', 'Would mediation fit {{APP_NAME}}?', 1, 1, 0, 0, true)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- 2. UPDATE SPINTAX TEMPLATES TO USE CORRECT TYPE
-- ============================================================================
-- The compose endpoint looks for templateType = 'initial_outreach'
-- Ensure our templates are correctly typed
-- FIXED (Jan 2026): Added proper parentheses to fix AND/OR precedence bug

UPDATE sa_spintax_templates
SET template_type = 'initial_outreach'
WHERE template_type = 'email' 
  AND (name LIKE '%Initial%' OR name LIKE '%Outreach%');

-- Also ensure at least one initial_outreach template exists
INSERT INTO sa_spintax_templates (
  name, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables,
  active
)
SELECT 
  'Default Initial Outreach',
  'initial_outreach',
  '{Quick question|Question} about {{APP_NAME}} {monetization|ad revenue}',
  E'{Hi|Hello|Hey} {{FIRST_NAME}},\n\n{I noticed|I came across|I saw} {{APP_NAME}} on the {App Store|Play Store} and {was impressed by|really liked} what you''ve built.\n\n{I work with|We help} app developers like {{COMPANY}} {increase their ad revenue|boost their monetization} through our {premium|advanced} mediation platform.\n\n{Would you be open to|Could we} {a quick call|chatting} about {how we might help|potential synergies}?\n\n{Best|Cheers|Thanks},\nApexMediation Team',
  '["FIRST_NAME", "APP_NAME", "COMPANY"]'::jsonb,
  true
WHERE NOT EXISTS (
  SELECT 1 FROM sa_spintax_templates WHERE template_type = 'initial_outreach'
);

-- ============================================================================
-- 3. ENSURE sa_autonomous_jobs HAS scheduled_at DEFAULT
-- ============================================================================
-- The scheduled_at column is NOT NULL but has no default, causing inserts to fail

ALTER TABLE sa_autonomous_jobs 
ALTER COLUMN scheduled_at SET DEFAULT CURRENT_TIMESTAMP;

-- ============================================================================
-- 4. ADD MISSING sa_bandit_trials TABLE (if not exists)
-- ============================================================================
-- Thompson Sampler records each trial selection here

CREATE TABLE IF NOT EXISTS sa_bandit_trials (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  arm_id UUID NOT NULL REFERENCES sa_bandit_arms(id) ON DELETE CASCADE,
  sampled_score DECIMAL(10,6),
  competing_arms_count INTEGER,
  success BOOLEAN,
  outcome_recorded_at TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_bandit_trials_arm_id ON sa_bandit_trials(arm_id);

-- ============================================================================
-- 5. ADD last_sampled_at AND retirement COLUMNS TO sa_bandit_arms IF MISSING
-- ============================================================================

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'sa_bandit_arms' AND column_name = 'last_sampled_at'
  ) THEN
    ALTER TABLE sa_bandit_arms ADD COLUMN last_sampled_at TIMESTAMP;
  END IF;
  
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'sa_bandit_arms' AND column_name = 'last_sampled_score'
  ) THEN
    ALTER TABLE sa_bandit_arms ADD COLUMN last_sampled_score DECIMAL(10,6);
  END IF;
  
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'sa_bandit_arms' AND column_name = 'retired_at'
  ) THEN
    ALTER TABLE sa_bandit_arms ADD COLUMN retired_at TIMESTAMP;
  END IF;
  
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'sa_bandit_arms' AND column_name = 'retirement_reason'
  ) THEN
    ALTER TABLE sa_bandit_arms ADD COLUMN retirement_reason TEXT;
  END IF;
END
$$;

-- ============================================================================
-- DONE
-- ============================================================================
-- After this migration:
--   - 5 subject_line arms are ready for Thompson Sampling
--   - Templates have correct type for compose endpoint
--   - scheduled_at has a default value
--   - sa_bandit_trials table exists for tracking
-- ============================================================================
