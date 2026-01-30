-- Migration: 057_unified_sales_controls.sql
-- Purpose: Add global kill switch, unified engagement events, and cross-system metrics
-- Date: 2026-01-28
-- 
-- Features:
--   1. Global kill switch for ALL outbound email sends (V1 + V2)
--   2. Unified engagement events table for cross-system learning
--   3. Standardized metrics views for dashboards

-- ============================================================================
-- 1. GLOBAL KILL SWITCH (Emergency stop for all outbound sends)
-- ============================================================================

-- System-wide controls table (extends sa_config pattern)
CREATE TABLE IF NOT EXISTS sales_system_controls (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Control key (unique identifier)
  control_key VARCHAR(100) UNIQUE NOT NULL,
  
  -- Control value and metadata
  control_value TEXT NOT NULL,
  control_type VARCHAR(20) DEFAULT 'boolean', -- 'boolean', 'integer', 'string', 'json'
  
  -- Who and when
  updated_by VARCHAR(255),
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  -- Audit trail
  previous_value TEXT,
  change_reason TEXT,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sales_system_controls_key ON sales_system_controls(control_key);

-- Insert default kill switch (OFF = sends allowed)
INSERT INTO sales_system_controls (control_key, control_value, control_type, updated_by, change_reason)
VALUES ('global_kill_switch', 'false', 'boolean', 'system', 'Initial setup - sends enabled')
ON CONFLICT (control_key) DO NOTHING;

-- Insert per-system controls
INSERT INTO sales_system_controls (control_key, control_value, control_type, updated_by, change_reason)
VALUES 
  ('v1_outbound_enabled', 'true', 'boolean', 'system', 'Initial setup - V1 outbound enabled'),
  ('v2_autonomous_enabled', 'true', 'boolean', 'system', 'Initial setup - V2 autonomous enabled'),
  ('max_daily_sends_global', '500', 'integer', 'system', 'Initial setup - global daily limit'),
  ('bounce_rate_threshold', '7', 'integer', 'system', 'Initial setup - pause at 7% bounce rate')
ON CONFLICT (control_key) DO NOTHING;

-- Kill switch audit log
CREATE TABLE IF NOT EXISTS sales_control_audit (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  control_key VARCHAR(100) NOT NULL,
  old_value TEXT,
  new_value TEXT NOT NULL,
  changed_by VARCHAR(255) NOT NULL,
  change_reason TEXT,
  changed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  ip_address VARCHAR(45),
  user_agent TEXT
);

CREATE INDEX IF NOT EXISTS idx_sales_control_audit_key ON sales_control_audit(control_key);
CREATE INDEX IF NOT EXISTS idx_sales_control_audit_time ON sales_control_audit(changed_at DESC);

-- Function to check if sends are allowed
CREATE OR REPLACE FUNCTION sales_sends_allowed()
RETURNS BOOLEAN AS $$
DECLARE
  kill_switch_active BOOLEAN;
BEGIN
  SELECT control_value::BOOLEAN INTO kill_switch_active
  FROM sales_system_controls
  WHERE control_key = 'global_kill_switch';
  
  -- If kill switch is TRUE (active), sends are NOT allowed
  -- If kill switch is FALSE (inactive) or not found, sends ARE allowed
  RETURN COALESCE(NOT kill_switch_active, TRUE);
END;
$$ LANGUAGE plpgsql;

-- Function to update control with audit
CREATE OR REPLACE FUNCTION update_sales_control(
  p_key VARCHAR(100),
  p_value TEXT,
  p_changed_by VARCHAR(255),
  p_reason TEXT DEFAULT NULL,
  p_ip VARCHAR(45) DEFAULT NULL
) RETURNS BOOLEAN AS $$
DECLARE
  old_val TEXT;
BEGIN
  -- Get current value
  SELECT control_value INTO old_val
  FROM sales_system_controls
  WHERE control_key = p_key;
  
  -- Update control
  UPDATE sales_system_controls
  SET 
    control_value = p_value,
    previous_value = old_val,
    updated_by = p_changed_by,
    change_reason = p_reason,
    updated_at = CURRENT_TIMESTAMP
  WHERE control_key = p_key;
  
  -- Log audit
  INSERT INTO sales_control_audit (control_key, old_value, new_value, changed_by, change_reason, ip_address)
  VALUES (p_key, old_val, p_value, p_changed_by, p_reason, p_ip);
  
  RETURN TRUE;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- 2. UNIFIED ENGAGEMENT EVENTS (Cross-system learning)
-- ============================================================================

-- Unified engagement events table - normalizes events from V1 and V2
CREATE TABLE IF NOT EXISTS unified_engagement_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Source system identification
  source_system VARCHAR(20) NOT NULL, -- 'v1_outbound', 'v2_autonomous', 'v1_trial_nurture'
  source_message_id UUID, -- Original message ID from source system
  source_contact_id UUID, -- Original contact ID from source system
  
  -- Normalized contact info
  contact_email VARCHAR(255) NOT NULL,
  contact_domain VARCHAR(255), -- Extracted domain for domain-level analytics
  
  -- Event details
  event_type VARCHAR(50) NOT NULL, -- 'sent', 'delivered', 'opened', 'clicked', 'replied', 'bounced', 'complained', 'unsubscribed', 'converted'
  event_timestamp TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  
  -- Message context (for learning)
  subject_line TEXT,
  template_id UUID,
  template_name VARCHAR(255),
  bandit_arm_id UUID, -- For Thompson Sampler learning (V2)
  sequence_name VARCHAR(255), -- For V1 sequence attribution
  step_number INTEGER,
  
  -- Engagement metadata
  event_metadata JSONB DEFAULT '{}',
  -- Examples:
  -- { "link_clicked": "pricing", "time_to_open_seconds": 3600 }
  -- { "bounce_type": "hard", "bounce_reason": "invalid" }
  -- { "reply_sentiment": "positive", "reply_classification": "interested" }
  
  -- Attribution
  campaign_id UUID,
  campaign_name VARCHAR(255),
  
  -- Outcome tracking (for conversion attribution)
  is_positive_outcome BOOLEAN GENERATED ALWAYS AS (
    event_type IN ('opened', 'clicked', 'replied', 'converted')
  ) STORED,
  
  -- Deduplication
  event_hash VARCHAR(64) UNIQUE, -- SHA-256 of (contact_email + event_type + source_message_id + DATE(event_timestamp))
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Indexes for analytics and learning
CREATE INDEX IF NOT EXISTS idx_unified_events_email ON unified_engagement_events(contact_email);
CREATE INDEX IF NOT EXISTS idx_unified_events_domain ON unified_engagement_events(contact_domain);
CREATE INDEX IF NOT EXISTS idx_unified_events_type ON unified_engagement_events(event_type);
CREATE INDEX IF NOT EXISTS idx_unified_events_source ON unified_engagement_events(source_system);
CREATE INDEX IF NOT EXISTS idx_unified_events_timestamp ON unified_engagement_events(event_timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_unified_events_template ON unified_engagement_events(template_id);
CREATE INDEX IF NOT EXISTS idx_unified_events_arm ON unified_engagement_events(bandit_arm_id) WHERE bandit_arm_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_unified_events_positive ON unified_engagement_events(is_positive_outcome) WHERE is_positive_outcome = TRUE;

-- Function to record unified event (with deduplication)
CREATE OR REPLACE FUNCTION record_unified_event(
  p_source_system VARCHAR(20),
  p_source_message_id UUID,
  p_source_contact_id UUID,
  p_contact_email VARCHAR(255),
  p_event_type VARCHAR(50),
  p_subject_line TEXT DEFAULT NULL,
  p_template_id UUID DEFAULT NULL,
  p_template_name VARCHAR(255) DEFAULT NULL,
  p_bandit_arm_id UUID DEFAULT NULL,
  p_sequence_name VARCHAR(255) DEFAULT NULL,
  p_step_number INTEGER DEFAULT NULL,
  p_event_metadata JSONB DEFAULT '{}',
  p_campaign_id UUID DEFAULT NULL,
  p_campaign_name VARCHAR(255) DEFAULT NULL
) RETURNS UUID AS $$
DECLARE
  v_event_hash VARCHAR(64);
  v_contact_domain VARCHAR(255);
  v_new_id UUID;
BEGIN
  -- Extract domain from email
  v_contact_domain := SPLIT_PART(p_contact_email, '@', 2);
  
  -- Generate deduplication hash
  v_event_hash := encode(sha256(
    (COALESCE(p_contact_email, '') || '|' || 
     COALESCE(p_event_type, '') || '|' || 
     COALESCE(p_source_message_id::TEXT, '') || '|' ||
     COALESCE(DATE(CURRENT_TIMESTAMP)::TEXT, ''))::BYTEA
  ), 'hex');
  
  -- Insert with conflict handling
  INSERT INTO unified_engagement_events (
    source_system, source_message_id, source_contact_id,
    contact_email, contact_domain, event_type,
    subject_line, template_id, template_name,
    bandit_arm_id, sequence_name, step_number,
    event_metadata, campaign_id, campaign_name,
    event_hash
  ) VALUES (
    p_source_system, p_source_message_id, p_source_contact_id,
    p_contact_email, v_contact_domain, p_event_type,
    p_subject_line, p_template_id, p_template_name,
    p_bandit_arm_id, p_sequence_name, p_step_number,
    p_event_metadata, p_campaign_id, p_campaign_name,
    v_event_hash
  )
  ON CONFLICT (event_hash) DO NOTHING
  RETURNING id INTO v_new_id;
  
  RETURN v_new_id;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- 3. STANDARDIZED METRICS VIEWS (Cross-system dashboards)
-- ============================================================================

-- Daily send metrics (combined V1 + V2)
CREATE OR REPLACE VIEW unified_daily_metrics AS
SELECT 
  DATE(event_timestamp) as metric_date,
  source_system,
  COUNT(*) FILTER (WHERE event_type = 'sent') as emails_sent,
  COUNT(*) FILTER (WHERE event_type = 'delivered') as emails_delivered,
  COUNT(*) FILTER (WHERE event_type = 'opened') as emails_opened,
  COUNT(*) FILTER (WHERE event_type = 'clicked') as emails_clicked,
  COUNT(*) FILTER (WHERE event_type = 'replied') as emails_replied,
  COUNT(*) FILTER (WHERE event_type = 'bounced') as emails_bounced,
  COUNT(*) FILTER (WHERE event_type = 'complained') as emails_complained,
  COUNT(*) FILTER (WHERE event_type = 'unsubscribed') as emails_unsubscribed,
  COUNT(*) FILTER (WHERE event_type = 'converted') as conversions,
  -- Calculated rates (avoid division by zero)
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'opened')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as open_rate,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'clicked')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as click_rate,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'replied')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as reply_rate,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'bounced')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as bounce_rate
FROM unified_engagement_events
GROUP BY DATE(event_timestamp), source_system
ORDER BY metric_date DESC, source_system;

-- Combined totals view (all systems aggregated)
CREATE OR REPLACE VIEW unified_metrics_totals AS
SELECT 
  'all_systems' as source_system,
  COUNT(*) FILTER (WHERE event_type = 'sent') as total_sent,
  COUNT(*) FILTER (WHERE event_type = 'opened') as total_opened,
  COUNT(*) FILTER (WHERE event_type = 'clicked') as total_clicked,
  COUNT(*) FILTER (WHERE event_type = 'replied') as total_replied,
  COUNT(*) FILTER (WHERE event_type = 'bounced') as total_bounced,
  COUNT(*) FILTER (WHERE event_type = 'converted') as total_converted,
  COUNT(DISTINCT contact_email) as unique_contacts,
  COUNT(DISTINCT contact_domain) as unique_domains,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'opened')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as overall_open_rate,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'bounced')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as overall_bounce_rate
FROM unified_engagement_events;

-- Template/subject line performance (for A/B testing)
CREATE OR REPLACE VIEW unified_template_performance AS
SELECT 
  COALESCE(template_name, subject_line, 'Unknown') as template_identifier,
  source_system,
  COUNT(*) FILTER (WHERE event_type = 'sent') as times_sent,
  COUNT(*) FILTER (WHERE event_type = 'opened') as times_opened,
  COUNT(*) FILTER (WHERE event_type = 'clicked') as times_clicked,
  COUNT(*) FILTER (WHERE event_type = 'replied') as times_replied,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'opened')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as open_rate,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'replied')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as reply_rate
FROM unified_engagement_events
WHERE template_name IS NOT NULL OR subject_line IS NOT NULL
GROUP BY COALESCE(template_name, subject_line, 'Unknown'), source_system
HAVING COUNT(*) FILTER (WHERE event_type = 'sent') >= 5
ORDER BY times_sent DESC;

-- Domain health view (cross-system)
CREATE OR REPLACE VIEW unified_domain_health AS
SELECT 
  contact_domain,
  COUNT(*) FILTER (WHERE event_type = 'sent') as total_sent,
  COUNT(*) FILTER (WHERE event_type = 'bounced') as total_bounced,
  COUNT(*) FILTER (WHERE event_type = 'complained') as total_complained,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'bounced')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as bounce_rate,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'complained')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as complaint_rate,
  MAX(event_timestamp) as last_activity
FROM unified_engagement_events
GROUP BY contact_domain
HAVING COUNT(*) FILTER (WHERE event_type = 'sent') >= 3
ORDER BY bounce_rate DESC, total_sent DESC;

-- Thompson Sampler arm performance (V2 specific, but visible in unified view)
CREATE OR REPLACE VIEW unified_bandit_performance AS
SELECT 
  bandit_arm_id,
  COUNT(*) FILTER (WHERE event_type = 'sent') as trials,
  COUNT(*) FILTER (WHERE event_type IN ('opened', 'clicked', 'replied')) as successes,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type IN ('opened', 'clicked', 'replied'))::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as success_rate
FROM unified_engagement_events
WHERE bandit_arm_id IS NOT NULL
GROUP BY bandit_arm_id
ORDER BY trials DESC;

-- Today's sending status (for kill switch monitoring)
CREATE OR REPLACE VIEW unified_today_status AS
SELECT 
  source_system,
  COUNT(*) FILTER (WHERE event_type = 'sent') as sent_today,
  COUNT(*) FILTER (WHERE event_type = 'bounced') as bounced_today,
  CASE WHEN COUNT(*) FILTER (WHERE event_type = 'sent') > 0 
    THEN ROUND(COUNT(*) FILTER (WHERE event_type = 'bounced')::DECIMAL / 
         COUNT(*) FILTER (WHERE event_type = 'sent') * 100, 2)
    ELSE 0 
  END as bounce_rate_today,
  (SELECT control_value::BOOLEAN FROM sales_system_controls WHERE control_key = 'global_kill_switch') as kill_switch_active,
  (SELECT control_value::INTEGER FROM sales_system_controls WHERE control_key = 'max_daily_sends_global') as daily_limit
FROM unified_engagement_events
WHERE DATE(event_timestamp) = CURRENT_DATE
GROUP BY source_system;

-- ============================================================================
-- 4. SYNC FUNCTIONS (Backfill from existing systems)
-- ============================================================================

-- Function to sync V1 outbound emails to unified events
CREATE OR REPLACE FUNCTION sync_v1_outbound_to_unified()
RETURNS INTEGER AS $$
DECLARE
  synced_count INTEGER := 0;
BEGIN
  -- Sync sent events
  INSERT INTO unified_engagement_events (
    source_system, source_message_id, source_contact_id,
    contact_email, contact_domain, event_type, event_timestamp,
    subject_line, sequence_name, step_number, event_hash
  )
  SELECT 
    'v1_outbound',
    e.id,
    e.contact_id,
    e.to_email,
    SPLIT_PART(e.to_email, '@', 2),
    'sent',
    COALESCE(e.sent_at, e.created_at),
    e.subject,
    NULL,
    e.step_number,
    encode(sha256((e.to_email || '|sent|' || e.id::TEXT || '|' || DATE(COALESCE(e.sent_at, e.created_at))::TEXT)::BYTEA), 'hex')
  FROM outreach_emails e
  WHERE e.sent_at IS NOT NULL
  ON CONFLICT (event_hash) DO NOTHING;
  
  GET DIAGNOSTICS synced_count = ROW_COUNT;
  
  -- Sync opened events
  INSERT INTO unified_engagement_events (
    source_system, source_message_id, source_contact_id,
    contact_email, contact_domain, event_type, event_timestamp,
    subject_line, event_hash
  )
  SELECT 
    'v1_outbound',
    e.id,
    e.contact_id,
    e.to_email,
    SPLIT_PART(e.to_email, '@', 2),
    'opened',
    e.opened_at,
    e.subject,
    encode(sha256((e.to_email || '|opened|' || e.id::TEXT || '|' || DATE(e.opened_at)::TEXT)::BYTEA), 'hex')
  FROM outreach_emails e
  WHERE e.opened_at IS NOT NULL
  ON CONFLICT (event_hash) DO NOTHING;
  
  -- Sync clicked events
  INSERT INTO unified_engagement_events (
    source_system, source_message_id, source_contact_id,
    contact_email, contact_domain, event_type, event_timestamp,
    subject_line, event_hash
  )
  SELECT 
    'v1_outbound',
    e.id,
    e.contact_id,
    e.to_email,
    SPLIT_PART(e.to_email, '@', 2),
    'clicked',
    e.clicked_at,
    e.subject,
    encode(sha256((e.to_email || '|clicked|' || e.id::TEXT || '|' || DATE(e.clicked_at)::TEXT)::BYTEA), 'hex')
  FROM outreach_emails e
  WHERE e.clicked_at IS NOT NULL
  ON CONFLICT (event_hash) DO NOTHING;
  
  -- Sync bounced events
  INSERT INTO unified_engagement_events (
    source_system, source_message_id, source_contact_id,
    contact_email, contact_domain, event_type, event_timestamp,
    subject_line, event_hash
  )
  SELECT 
    'v1_outbound',
    e.id,
    e.contact_id,
    e.to_email,
    SPLIT_PART(e.to_email, '@', 2),
    'bounced',
    e.bounced_at,
    e.subject,
    encode(sha256((e.to_email || '|bounced|' || e.id::TEXT || '|' || DATE(e.bounced_at)::TEXT)::BYTEA), 'hex')
  FROM outreach_emails e
  WHERE e.bounced_at IS NOT NULL
  ON CONFLICT (event_hash) DO NOTHING;
  
  RETURN synced_count;
END;
$$ LANGUAGE plpgsql;

-- Function to sync V2 autonomous events to unified events
CREATE OR REPLACE FUNCTION sync_v2_autonomous_to_unified()
RETURNS INTEGER AS $$
DECLARE
  synced_count INTEGER := 0;
BEGIN
  -- Sync from sa_outbound_messages and sa_email_events
  INSERT INTO unified_engagement_events (
    source_system, source_message_id, source_contact_id,
    contact_email, contact_domain, event_type, event_timestamp,
    subject_line, bandit_arm_id, event_hash
  )
  SELECT 
    'v2_autonomous',
    om.id,
    om.contact_id,
    c.email,
    SPLIT_PART(c.email, '@', 2),
    CASE 
      WHEN om.sent_at IS NOT NULL AND om.opened_at IS NULL AND om.clicked_at IS NULL THEN 'sent'
      WHEN om.opened_at IS NOT NULL THEN 'opened'
      WHEN om.clicked_at IS NOT NULL THEN 'clicked'
      WHEN om.replied_at IS NOT NULL THEN 'replied'
      ELSE 'sent'
    END,
    COALESCE(om.sent_at, om.created_at),
    om.subject,
    om.subject_arm_id,
    encode(sha256((c.email || '|sent|' || om.id::TEXT || '|' || DATE(COALESCE(om.sent_at, om.created_at))::TEXT)::BYTEA), 'hex')
  FROM sa_outbound_messages om
  JOIN sa_contacts c ON c.id = om.contact_id
  WHERE om.sent_at IS NOT NULL
  ON CONFLICT (event_hash) DO NOTHING;
  
  GET DIAGNOSTICS synced_count = ROW_COUNT;
  
  RETURN synced_count;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- 5. GRANTS
-- ============================================================================

-- Grant usage to application roles (adjust role names as needed)
-- GRANT SELECT, INSERT, UPDATE ON sales_system_controls TO adproject;
-- GRANT SELECT, INSERT ON sales_control_audit TO adproject;
-- GRANT SELECT, INSERT ON unified_engagement_events TO adproject;
-- GRANT SELECT ON unified_daily_metrics TO adproject;
-- GRANT SELECT ON unified_metrics_totals TO adproject;
-- GRANT SELECT ON unified_template_performance TO adproject;
-- GRANT SELECT ON unified_domain_health TO adproject;
-- GRANT SELECT ON unified_bandit_performance TO adproject;
-- GRANT SELECT ON unified_today_status TO adproject;

-- Done
SELECT 'Migration 057_unified_sales_controls.sql completed successfully' as status;
