-- ============================================================================
-- Migration 054: Closed Loop Tracking
-- ============================================================================
-- Enables the Thompson Sampler to automatically learn from email responses.
-- This is the "brain" that makes the system self-optimizing over time.
--
-- The closed loop works as follows:
--   1. When we send an email, we record which subject_arm_id was used
--   2. When we receive a response (webhook), we look up the arm
--   3. If positive response (INTERESTED/MEETING), we call recordOutcome(arm, true)
--   4. If negative response (BOUNCE/UNSUBSCRIBE), we call recordOutcome(arm, false)
--   5. Thompson Sampler automatically shifts traffic to winning arms
-- ============================================================================

-- ============================================================================
-- 1. OUTBOUND MESSAGES TABLE (Tracks sent emails + arm used)
-- ============================================================================
CREATE TABLE IF NOT EXISTS sa_outbound_messages (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID NOT NULL REFERENCES sa_contacts(id) ON DELETE CASCADE,
  
  -- Content
  subject TEXT NOT NULL,
  body_text TEXT NOT NULL,
  body_html TEXT,
  
  -- Thompson Sampler tracking (THE KEY FOR CLOSED LOOP)
  subject_arm_id UUID REFERENCES sa_bandit_arms(id),
  template_id UUID,
  variation_hash VARCHAR(64),
  
  -- Status
  status VARCHAR(20) DEFAULT 'pending', -- pending, sent, delivered, opened, clicked, replied, bounced
  sent_at TIMESTAMP,
  delivered_at TIMESTAMP,
  opened_at TIMESTAMP,
  clicked_at TIMESTAMP,
  replied_at TIMESTAMP,
  
  -- Metadata
  message_id VARCHAR(255), -- External ID from email provider (SendGrid, etc)
  provider VARCHAR(50), -- sendgrid, resend, mailgun
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_outbound_contact ON sa_outbound_messages(contact_id);
CREATE INDEX IF NOT EXISTS idx_sa_outbound_arm ON sa_outbound_messages(subject_arm_id);
CREATE INDEX IF NOT EXISTS idx_sa_outbound_status ON sa_outbound_messages(status);
CREATE INDEX IF NOT EXISTS idx_sa_outbound_sent ON sa_outbound_messages(sent_at);

-- ============================================================================
-- 2. EMAIL EVENTS TABLE (Tracks webhook events for analytics)
-- ============================================================================
CREATE TABLE IF NOT EXISTS sa_email_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  outbound_message_id UUID REFERENCES sa_outbound_messages(id) ON DELETE CASCADE,
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  
  -- Event details
  event_type VARCHAR(50) NOT NULL, -- open, click, bounce, unsubscribe, reply
  event_data JSONB DEFAULT '{}',
  
  -- Bandit feedback
  arm_id UUID REFERENCES sa_bandit_arms(id),
  outcome_recorded BOOLEAN DEFAULT false,
  
  -- Source
  source VARCHAR(50), -- sendgrid, mailgun, manual
  raw_payload JSONB,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_email_events_outbound ON sa_email_events(outbound_message_id);
CREATE INDEX IF NOT EXISTS idx_sa_email_events_contact ON sa_email_events(contact_id);
CREATE INDEX IF NOT EXISTS idx_sa_email_events_type ON sa_email_events(event_type);
CREATE INDEX IF NOT EXISTS idx_sa_email_events_arm ON sa_email_events(arm_id);

-- ============================================================================
-- 3. UPDATE sa_inbox_messages TO LINK BACK TO OUTBOUND
-- ============================================================================
-- Add column to track which outbound message triggered the response
ALTER TABLE sa_inbox_messages 
ADD COLUMN IF NOT EXISTS original_outbound_id UUID REFERENCES sa_outbound_messages(id);

-- ============================================================================
-- 4. COMPETITOR DETECTION CACHE (For Sniper Upgrade)
-- ============================================================================
-- Stores which competitors were detected for each contact
-- This enables quick filtering and reporting
CREATE TABLE IF NOT EXISTS sa_competitor_detections (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID NOT NULL REFERENCES sa_contacts(id) ON DELETE CASCADE,
  
  -- Competitor info
  competitor_domain VARCHAR(255) NOT NULL,
  competitor_name VARCHAR(100),
  competitor_tier INTEGER DEFAULT 3, -- 1=highest priority, 3=lowest
  
  -- Detection source
  detected_in VARCHAR(50) DEFAULT 'app_ads_txt', -- app_ads_txt, sdk_scan, manual
  detection_confidence INTEGER DEFAULT 100,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(contact_id, competitor_domain)
);

CREATE INDEX IF NOT EXISTS idx_sa_competitor_contact ON sa_competitor_detections(contact_id);
CREATE INDEX IF NOT EXISTS idx_sa_competitor_domain ON sa_competitor_detections(competitor_domain);
CREATE INDEX IF NOT EXISTS idx_sa_competitor_tier ON sa_competitor_detections(competitor_tier);

-- ============================================================================
-- 5. ADD COMPETITOR COLUMNS TO sa_contacts FOR QUICK ACCESS
-- ============================================================================
ALTER TABLE sa_contacts
ADD COLUMN IF NOT EXISTS has_competitor BOOLEAN DEFAULT false,
ADD COLUMN IF NOT EXISTS top_competitor VARCHAR(100),
ADD COLUMN IF NOT EXISTS competitor_tier INTEGER;

-- ============================================================================
-- 6. SEED ADDITIONAL HIGH-VALUE SOURCES
-- ============================================================================
-- Add sources specifically targeting competitor users
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours)
VALUES 
  -- Games with "Ads" in description (monetization intent)
  ('itunes', 'Casual Games with Ads - US', 'casual game free ads', 'Games', 'us', 'active', 12),
  ('itunes', 'Puzzle Games with Ads - US', 'puzzle game free', 'Games', 'us', 'active', 12),
  ('itunes', 'Arcade Games - US', 'arcade game mobile', 'Games', 'us', 'active', 12),
  ('itunes', 'Word Games - US', 'word puzzle', 'Games', 'us', 'active', 24),
  ('itunes', 'Card Games - US', 'card game solitaire', 'Games', 'us', 'active', 24),
  -- International markets (high ad revenue)
  ('itunes', 'Casual Games - DE', 'spiel kostenlos', 'Games', 'de', 'active', 24),
  ('itunes', 'Casual Games - JP', 'ゲーム 無料', 'Games', 'jp', 'active', 24)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- 7. SEED ADDITIONAL BANDIT ARMS (More subject line variations)
-- ============================================================================
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, total_trials, total_successes, active)
VALUES 
  -- Direct value proposition
  ('subject_line', 'revenue_boost', 'Boost {{APP_NAME}} revenue by 30%+', 1, 1, 0, 0, true),
  ('subject_line', 'competitor_switch', 'Why developers switch from IronSource/Unity to us', 1, 1, 0, 0, true),
  -- Social proof
  ('subject_line', 'social_proof', 'How similar apps doubled their eCPM', 1, 1, 0, 0, true),
  ('subject_line', 'case_study', 'Case study: 47% revenue increase for {{APP_GENRE}} apps', 1, 1, 0, 0, true),
  -- Urgency/scarcity
  ('subject_line', 'limited_onboarding', 'Limited spots for {{APP_GENRE}} apps this month', 1, 1, 0, 0, true),
  -- Curiosity
  ('subject_line', 'one_change', 'The one SDK change that doubled ad revenue', 1, 1, 0, 0, true)
ON CONFLICT DO NOTHING;
