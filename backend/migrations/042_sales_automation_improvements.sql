-- Migration: Sales Automation Improvements
-- Adds tables for email warmup, verification details, and engagement branching

-- ============================================================================
-- EMAIL WARMUP TRACKING
-- ============================================================================

-- Track mailbox warmup status
CREATE TABLE IF NOT EXISTS email_mailbox_warmup (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  mailbox VARCHAR(255) NOT NULL UNIQUE,
  
  -- Warmup tracking
  warmup_start_date DATE NOT NULL DEFAULT CURRENT_DATE,
  current_phase INTEGER DEFAULT 1,
  reputation_status VARCHAR(20) DEFAULT 'warming', -- 'warming', 'warm', 'cold', 'damaged'
  
  -- Daily limits
  daily_sent INTEGER DEFAULT 0,
  daily_limit INTEGER DEFAULT 5,
  
  -- Lifetime metrics
  total_sent INTEGER DEFAULT 0,
  bounce_count INTEGER DEFAULT 0,
  spam_count INTEGER DEFAULT 0,
  open_count INTEGER DEFAULT 0,
  reply_count INTEGER DEFAULT 0,
  
  -- Timestamps
  last_warmup_at TIMESTAMP,
  last_cold_send_at TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_mailbox_warmup_status ON email_mailbox_warmup(reputation_status);

-- ============================================================================
-- ENHANCED CONTACT VERIFICATION
-- ============================================================================

-- Add verification details column if not exists
DO $$ 
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'sales_contacts' AND column_name = 'verification_details'
  ) THEN
    ALTER TABLE sales_contacts ADD COLUMN verification_details JSONB DEFAULT '{}';
  END IF;
END $$;

-- ============================================================================
-- ENHANCED OUTREACH TRACKING
-- ============================================================================

-- Add scheduling columns to outreach_emails
DO $$ 
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'outreach_emails' AND column_name = 'scheduling_reason'
  ) THEN
    ALTER TABLE outreach_emails ADD COLUMN scheduling_reason TEXT;
    ALTER TABLE outreach_emails ADD COLUMN scheduling_score INTEGER;
    ALTER TABLE outreach_emails ADD COLUMN send_timezone VARCHAR(50);
  END IF;
END $$;

-- Add branching columns to outreach_enrollments
DO $$ 
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns 
    WHERE table_name = 'outreach_enrollments' AND column_name = 'branch_reason'
  ) THEN
    ALTER TABLE outreach_enrollments ADD COLUMN branch_reason TEXT;
    ALTER TABLE outreach_enrollments ADD COLUMN branches_count INTEGER DEFAULT 0;
    ALTER TABLE outreach_enrollments ADD COLUMN completion_reason TEXT;
    ALTER TABLE outreach_enrollments ADD COLUMN pause_reason TEXT;
    ALTER TABLE outreach_enrollments ADD COLUMN engagement_score INTEGER DEFAULT 0;
  END IF;
END $$;

-- ============================================================================
-- COMPANY DEDUPLICATION
-- ============================================================================

-- Add deduplication tracking
CREATE TABLE IF NOT EXISTS company_deduplication (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  master_company_id UUID REFERENCES sales_companies(id),
  duplicate_company_id UUID REFERENCES sales_companies(id),
  
  -- Matching info
  match_type VARCHAR(50), -- 'domain', 'name_fuzzy', 'email_pattern', 'manual'
  match_score DECIMAL(5,2),
  verified BOOLEAN DEFAULT false,
  
  -- Action taken
  merged BOOLEAN DEFAULT false,
  merged_at TIMESTAMP,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(master_company_id, duplicate_company_id)
);

-- ============================================================================
-- ENGAGEMENT BRANCHING SEQUENCES (DEFAULT)
-- ============================================================================

-- Insert default branch sequences if not exist
INSERT INTO outreach_sequences (id, name, description, total_steps, target_segment, active)
VALUES 
  (gen_random_uuid(), 'Warm Hesitant - Social Proof', 'For prospects who open but don''t reply', 2, 'all', true),
  (gen_random_uuid(), 'Hot Not Ready - Case Study', 'For prospects clicking links but not replying', 1, 'all', true),
  (gen_random_uuid(), 'Breakup Sequence', 'Final attempt for unresponsive prospects', 1, 'all', true),
  (gen_random_uuid(), 'Re-engagement Sequence', 'For contacts who went cold after initial engagement', 1, 'all', true)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- PERFORMANCE ANALYTICS
-- ============================================================================

-- Add send timing analytics
CREATE TABLE IF NOT EXISTS send_timing_analytics (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Time bucket
  day_of_week SMALLINT NOT NULL, -- 0-6
  hour_of_day SMALLINT NOT NULL, -- 0-23
  timezone VARCHAR(50) DEFAULT 'UTC',
  
  -- Metrics
  emails_sent INTEGER DEFAULT 0,
  emails_opened INTEGER DEFAULT 0,
  emails_clicked INTEGER DEFAULT 0,
  emails_replied INTEGER DEFAULT 0,
  
  -- Calculated rates (updated by analytics job)
  open_rate DECIMAL(5,2),
  click_rate DECIMAL(5,2),
  reply_rate DECIMAL(5,2),
  
  -- Time period
  period_start DATE NOT NULL,
  period_end DATE NOT NULL,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(day_of_week, hour_of_day, timezone, period_start)
);

-- ============================================================================
-- HELPER FUNCTIONS
-- ============================================================================

-- Function to check if mailbox can send cold emails
CREATE OR REPLACE FUNCTION can_mailbox_send_cold(p_mailbox VARCHAR)
RETURNS TABLE (
  allowed BOOLEAN,
  reason TEXT,
  remaining_limit INTEGER
) AS $$
DECLARE
  v_warmup RECORD;
  v_current_day INTEGER;
  v_phase_limit INTEGER;
BEGIN
  SELECT * INTO v_warmup FROM email_mailbox_warmup WHERE mailbox = p_mailbox;
  
  IF NOT FOUND THEN
    RETURN QUERY SELECT false, 'Mailbox not initialized for warmup'::TEXT, 0;
    RETURN;
  END IF;
  
  IF v_warmup.reputation_status = 'damaged' THEN
    RETURN QUERY SELECT false, 'Mailbox reputation is damaged'::TEXT, 0;
    RETURN;
  END IF;
  
  v_current_day := (CURRENT_DATE - v_warmup.warmup_start_date) + 1;
  
  IF v_current_day < 11 THEN
    RETURN QUERY SELECT false, 
      format('Warmup in progress: Day %s/11', v_current_day)::TEXT, 0;
    RETURN;
  END IF;
  
  -- Calculate phase limit based on day
  v_phase_limit := CASE 
    WHEN v_current_day >= 28 THEN 100
    WHEN v_current_day >= 21 THEN 80
    WHEN v_current_day >= 15 THEN 70
    WHEN v_current_day >= 14 THEN 65
    ELSE 50
  END;
  
  -- Check bounce rate
  IF v_warmup.total_sent > 10 AND 
     v_warmup.bounce_count::float / v_warmup.total_sent > 0.05 THEN
    RETURN QUERY SELECT false, 
      format('Bounce rate too high: %s%%', 
        ROUND(v_warmup.bounce_count::float / v_warmup.total_sent * 100, 1))::TEXT, 0;
    RETURN;
  END IF;
  
  -- Check spam rate
  IF v_warmup.total_sent > 10 AND 
     v_warmup.spam_count::float / v_warmup.total_sent > 0.01 THEN
    RETURN QUERY SELECT false, 
      format('Spam rate too high: %s%%', 
        ROUND(v_warmup.spam_count::float / v_warmup.total_sent * 100, 1))::TEXT, 0;
    RETURN;
  END IF;
  
  RETURN QUERY SELECT true, 'OK'::TEXT, 
    GREATEST(0, v_phase_limit - v_warmup.daily_sent - 10);
END;
$$ LANGUAGE plpgsql;

-- Function to get engagement level for an enrollment
CREATE OR REPLACE FUNCTION get_enrollment_engagement_level(p_enrollment_id UUID)
RETURNS VARCHAR AS $$
DECLARE
  v_stats RECORD;
  v_open_rate DECIMAL;
BEGIN
  SELECT 
    COUNT(*) as sent,
    COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as opened,
    COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as clicked,
    COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as replied
  INTO v_stats
  FROM outreach_emails
  WHERE enrollment_id = p_enrollment_id
    AND status IN ('sent', 'delivered', 'opened', 'clicked', 'replied');
  
  IF v_stats.replied > 0 THEN
    RETURN 'hot';
  END IF;
  
  IF v_stats.sent > 0 THEN
    v_open_rate := v_stats.opened::decimal / v_stats.sent;
  ELSE
    v_open_rate := 0;
  END IF;
  
  IF v_stats.clicked >= 2 OR v_open_rate >= 0.75 THEN
    RETURN 'hot';
  END IF;
  
  IF v_stats.opened > 0 THEN
    RETURN 'warm';
  END IF;
  
  IF v_stats.sent >= 4 AND v_stats.opened = 0 THEN
    RETURN 'unresponsive';
  END IF;
  
  RETURN 'cold';
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- SCHEDULED JOB: Reset daily warmup counts
-- ============================================================================

-- This should be called by a cron job at midnight UTC
CREATE OR REPLACE FUNCTION reset_daily_warmup_counts()
RETURNS void AS $$
BEGIN
  UPDATE email_mailbox_warmup
  SET daily_sent = 0,
      updated_at = CURRENT_TIMESTAMP;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- VIEWS
-- ============================================================================

-- View for enrollment engagement status
CREATE OR REPLACE VIEW v_enrollment_engagement AS
SELECT 
  e.id,
  e.contact_id,
  e.sequence_id,
  e.status,
  e.current_step,
  e.branches_count,
  get_enrollment_engagement_level(e.id) as engagement_level,
  (SELECT COUNT(*) FROM outreach_emails WHERE enrollment_id = e.id AND status = 'sent') as emails_sent,
  (SELECT COUNT(*) FROM outreach_emails WHERE enrollment_id = e.id AND opened_at IS NOT NULL) as emails_opened,
  (SELECT COUNT(*) FROM outreach_emails WHERE enrollment_id = e.id AND clicked_at IS NOT NULL) as emails_clicked,
  (SELECT COUNT(*) FROM outreach_emails WHERE enrollment_id = e.id AND replied_at IS NOT NULL) as emails_replied,
  e.created_at,
  e.updated_at
FROM outreach_enrollments e;

-- View for mailbox health
CREATE OR REPLACE VIEW v_mailbox_health AS
SELECT 
  mailbox,
  reputation_status,
  (CURRENT_DATE - warmup_start_date) + 1 as warmup_day,
  daily_sent,
  total_sent,
  CASE WHEN total_sent > 0 
    THEN ROUND(bounce_count::decimal / total_sent * 100, 2) 
    ELSE 0 
  END as bounce_rate,
  CASE WHEN total_sent > 0 
    THEN ROUND(spam_count::decimal / total_sent * 100, 2) 
    ELSE 0 
  END as spam_rate,
  CASE WHEN total_sent > 0 
    THEN ROUND(open_count::decimal / total_sent * 100, 2) 
    ELSE 0 
  END as open_rate,
  (SELECT * FROM can_mailbox_send_cold(mailbox)) as cold_status,
  last_warmup_at,
  last_cold_send_at
FROM email_mailbox_warmup;
