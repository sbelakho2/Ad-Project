-- Migration: 058_unified_suppression_list.sql
-- Purpose: Create a unified suppression list for all sales automation systems
-- 
-- This creates a single source of truth for email addresses that should NEVER
-- receive outbound emails from ANY system (V1 or V2). Includes:
-- - Hard bounces (email doesn't exist)
-- - Spam complaints
-- - Manual opt-outs
-- - Compliance removals (GDPR, CAN-SPAM)
-- - Competitor domains

-- ============================================================================
-- UNIFIED SUPPRESSION LIST
-- ============================================================================

CREATE TABLE IF NOT EXISTS sales_suppression_list (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- What's suppressed
  email VARCHAR(512) NOT NULL,
  domain VARCHAR(255), -- Extracted for domain-level blocks
  
  -- Classification
  suppression_type VARCHAR(50) NOT NULL CHECK (suppression_type IN (
    'hard_bounce',      -- Email doesn't exist
    'soft_bounce',      -- Temporary issue (3 strikes = permanent)
    'spam_complaint',   -- Marked as spam by recipient
    'unsubscribe',      -- User requested removal
    'gdpr_erasure',     -- GDPR Article 17 request
    'manual_block',     -- Operator-added block
    'competitor',       -- Competitor domain
    'role_based',       -- role@ addresses (info@, sales@, etc.)
    'invalid_format',   -- Failed email validation
    'honeypot'          -- Known spam trap
  )),
  
  -- Source tracking
  source_system VARCHAR(50) NOT NULL CHECK (source_system IN (
    'v1_outbound',
    'v2_autonomous', 
    'webhook_bounce',
    'webhook_complaint',
    'manual_operator',
    'sync_import'
  )),
  source_message_id VARCHAR(255),     -- Original message that triggered
  source_contact_id UUID,             -- Original contact if known
  
  -- Metadata
  reason TEXT,                        -- Human-readable reason
  evidence JSONB DEFAULT '{}',        -- Original bounce/complaint data
  
  -- Auto-expiry for soft bounces
  expires_at TIMESTAMPTZ,             -- NULL = permanent
  soft_bounce_count INT DEFAULT 1,    -- Incremented on repeat soft bounces
  
  -- Audit
  created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  created_by VARCHAR(255),            -- Operator or system
  updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Unique constraint: one email per suppression type
CREATE UNIQUE INDEX IF NOT EXISTS idx_suppression_email_type 
  ON sales_suppression_list(LOWER(email), suppression_type);

-- Fast lookup for send checks  
CREATE INDEX IF NOT EXISTS idx_suppression_email_lookup 
  ON sales_suppression_list(LOWER(email)) 
  WHERE expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP;

-- Domain-level lookups (for competitor blocking)
CREATE INDEX IF NOT EXISTS idx_suppression_domain 
  ON sales_suppression_list(LOWER(domain))
  WHERE domain IS NOT NULL AND suppression_type = 'competitor';

-- Audit/reporting
CREATE INDEX IF NOT EXISTS idx_suppression_created 
  ON sales_suppression_list(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_suppression_type 
  ON sales_suppression_list(suppression_type);

-- ============================================================================
-- SUPPRESSION CHECK FUNCTION
-- ============================================================================

-- Fast check if an email is suppressed (used before every send)
CREATE OR REPLACE FUNCTION is_email_suppressed(check_email TEXT)
RETURNS TABLE (
  is_suppressed BOOLEAN,
  suppression_type VARCHAR(50),
  reason TEXT,
  suppressed_at TIMESTAMPTZ
) AS $$
BEGIN
  RETURN QUERY
  SELECT 
    TRUE,
    s.suppression_type,
    s.reason,
    s.created_at
  FROM sales_suppression_list s
  WHERE LOWER(s.email) = LOWER(check_email)
    AND (s.expires_at IS NULL OR s.expires_at > CURRENT_TIMESTAMP)
  ORDER BY s.created_at DESC
  LIMIT 1;
  
  -- If no rows returned, return not suppressed
  IF NOT FOUND THEN
    RETURN QUERY SELECT FALSE::BOOLEAN, NULL::VARCHAR(50), NULL::TEXT, NULL::TIMESTAMPTZ;
  END IF;
END;
$$ LANGUAGE plpgsql STABLE;

-- Check if a domain is blocked (competitor domains)
CREATE OR REPLACE FUNCTION is_domain_blocked(check_domain TEXT)
RETURNS BOOLEAN AS $$
BEGIN
  RETURN EXISTS (
    SELECT 1 FROM sales_suppression_list 
    WHERE LOWER(domain) = LOWER(check_domain)
      AND suppression_type = 'competitor'
      AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)
  );
END;
$$ LANGUAGE plpgsql STABLE;

-- ============================================================================
-- ADD TO SUPPRESSION LIST FUNCTION
-- ============================================================================

CREATE OR REPLACE FUNCTION add_to_suppression_list(
  p_email TEXT,
  p_type VARCHAR(50),
  p_source_system VARCHAR(50),
  p_reason TEXT DEFAULT NULL,
  p_evidence JSONB DEFAULT '{}',
  p_created_by VARCHAR(255) DEFAULT 'system',
  p_source_message_id VARCHAR(255) DEFAULT NULL,
  p_source_contact_id UUID DEFAULT NULL
) RETURNS UUID AS $$
DECLARE
  v_id UUID;
  v_domain TEXT;
  v_expires_at TIMESTAMPTZ;
BEGIN
  -- Extract domain from email
  v_domain := LOWER(SPLIT_PART(p_email, '@', 2));
  
  -- Soft bounces expire after 30 days (3 strikes = permanent)
  IF p_type = 'soft_bounce' THEN
    v_expires_at := CURRENT_TIMESTAMP + INTERVAL '30 days';
  END IF;
  
  -- Upsert: if exists for same email+type, update
  INSERT INTO sales_suppression_list (
    email, domain, suppression_type, source_system,
    source_message_id, source_contact_id,
    reason, evidence, expires_at, created_by
  ) VALUES (
    LOWER(p_email), v_domain, p_type, p_source_system,
    p_source_message_id, p_source_contact_id,
    p_reason, p_evidence, v_expires_at, p_created_by
  )
  ON CONFLICT (LOWER(email), suppression_type) DO UPDATE SET
    soft_bounce_count = CASE 
      WHEN p_type = 'soft_bounce' THEN sales_suppression_list.soft_bounce_count + 1
      ELSE sales_suppression_list.soft_bounce_count
    END,
    -- Convert to permanent if 3+ soft bounces
    expires_at = CASE 
      WHEN p_type = 'soft_bounce' AND sales_suppression_list.soft_bounce_count >= 2 THEN NULL
      ELSE EXCLUDED.expires_at
    END,
    -- Upgrade suppression type if more severe
    suppression_type = CASE
      WHEN p_type = 'hard_bounce' AND sales_suppression_list.suppression_type = 'soft_bounce' THEN 'hard_bounce'
      ELSE sales_suppression_list.suppression_type
    END,
    evidence = sales_suppression_list.evidence || p_evidence,
    updated_at = CURRENT_TIMESTAMP
  RETURNING id INTO v_id;
  
  RETURN v_id;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- SYNC EXISTING OPT-OUTS TO SUPPRESSION LIST
-- ============================================================================

-- Migrate existing opted-out contacts to suppression list
INSERT INTO sales_suppression_list (email, domain, suppression_type, source_system, reason, created_by, created_at)
SELECT 
  LOWER(email),
  LOWER(SPLIT_PART(email, '@', 2)),
  'unsubscribe',
  'sync_import',
  'Migrated from sa_contacts.status=opted_out',
  'migration_058',
  COALESCE(opted_out_at, CURRENT_TIMESTAMP)
FROM sa_contacts
WHERE status = 'opted_out' AND email IS NOT NULL
ON CONFLICT (LOWER(email), suppression_type) DO NOTHING;

-- ============================================================================
-- SAFE MODE CONFIGURATION TABLE
-- ============================================================================

-- Configuration for safe mode auto-throttle
CREATE TABLE IF NOT EXISTS sales_safe_mode_config (
  id SERIAL PRIMARY KEY,
  
  -- Thresholds that trigger safe mode
  bounce_rate_warning NUMERIC(5,2) DEFAULT 5.0,    -- %
  bounce_rate_critical NUMERIC(5,2) DEFAULT 7.0,   -- %
  complaint_rate_warning NUMERIC(5,3) DEFAULT 0.3, -- %
  complaint_rate_critical NUMERIC(5,3) DEFAULT 0.5,-- %
  
  -- Safe mode behaviors
  warning_throttle_percent INT DEFAULT 50,         -- Reduce sends by 50% on warning
  critical_pause_minutes INT DEFAULT 60,           -- Pause for 60 min on critical
  
  -- Auto-recovery
  recovery_check_interval_minutes INT DEFAULT 15,
  
  -- Audit
  created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Insert default config
INSERT INTO sales_safe_mode_config DEFAULT VALUES
ON CONFLICT DO NOTHING;

-- ============================================================================
-- SAFE MODE STATE TABLE
-- ============================================================================

CREATE TABLE IF NOT EXISTS sales_safe_mode_state (
  id SERIAL PRIMARY KEY,
  
  -- Current state
  mode VARCHAR(20) NOT NULL DEFAULT 'normal' CHECK (mode IN ('normal', 'warning', 'critical', 'paused')),
  
  -- Metrics that triggered the state
  current_bounce_rate NUMERIC(5,2) DEFAULT 0,
  current_complaint_rate NUMERIC(5,3) DEFAULT 0,
  emails_sent_7d INT DEFAULT 0,
  
  -- Throttle state
  throttle_percent INT DEFAULT 100,  -- 100 = no throttle
  
  -- State change tracking
  mode_changed_at TIMESTAMPTZ,
  mode_changed_by VARCHAR(255),
  mode_change_reason TEXT,
  
  -- Auto-recovery
  auto_recovery_at TIMESTAMPTZ,      -- When to check for recovery
  
  -- Timestamps
  created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Insert default state
INSERT INTO sales_safe_mode_state (mode, throttle_percent) 
VALUES ('normal', 100)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- SAFE MODE CHECK FUNCTION
-- ============================================================================

-- Check if sending is allowed and at what throttle level
CREATE OR REPLACE FUNCTION check_safe_mode()
RETURNS TABLE (
  can_send BOOLEAN,
  current_mode VARCHAR(20),
  throttle_percent INT,
  reason TEXT
) AS $$
DECLARE
  v_state RECORD;
BEGIN
  SELECT * INTO v_state FROM sales_safe_mode_state ORDER BY id LIMIT 1;
  
  IF v_state.mode = 'paused' THEN
    RETURN QUERY SELECT 
      FALSE, 
      v_state.mode, 
      0, 
      'System paused due to critical bounce/complaint rate'::TEXT;
  ELSIF v_state.mode = 'critical' THEN
    RETURN QUERY SELECT 
      FALSE, 
      v_state.mode, 
      0, 
      format('Critical threshold exceeded: bounce=%.1f%%, complaints=%.2f%%', 
             v_state.current_bounce_rate, v_state.current_complaint_rate)::TEXT;
  ELSIF v_state.mode = 'warning' THEN
    RETURN QUERY SELECT 
      TRUE, 
      v_state.mode, 
      v_state.throttle_percent, 
      format('Warning mode: throttled to %s%% (bounce=%.1f%%)', 
             v_state.throttle_percent, v_state.current_bounce_rate)::TEXT;
  ELSE
    RETURN QUERY SELECT TRUE, 'normal'::VARCHAR(20), 100, NULL::TEXT;
  END IF;
END;
$$ LANGUAGE plpgsql STABLE;

-- ============================================================================
-- UPDATE SAFE MODE FUNCTION
-- ============================================================================

CREATE OR REPLACE FUNCTION update_safe_mode_state()
RETURNS VOID AS $$
DECLARE
  v_config RECORD;
  v_stats RECORD;
  v_new_mode VARCHAR(20);
  v_throttle INT;
BEGIN
  -- Get config
  SELECT * INTO v_config FROM sales_safe_mode_config ORDER BY id LIMIT 1;
  
  -- Calculate current rates from unified events (last 7 days)
  SELECT 
    COALESCE(COUNT(*) FILTER (WHERE event_type = 'sent'), 0) as sent,
    COALESCE(COUNT(*) FILTER (WHERE event_type = 'bounced'), 0) as bounced,
    COALESCE(COUNT(*) FILTER (WHERE event_type = 'complained'), 0) as complained
  INTO v_stats
  FROM unified_engagement_events
  WHERE event_timestamp > CURRENT_TIMESTAMP - INTERVAL '7 days';
  
  -- Calculate rates
  DECLARE
    v_bounce_rate NUMERIC(5,2) := 0;
    v_complaint_rate NUMERIC(5,3) := 0;
  BEGIN
    IF v_stats.sent > 0 THEN
      v_bounce_rate := (v_stats.bounced::NUMERIC / v_stats.sent * 100);
      v_complaint_rate := (v_stats.complained::NUMERIC / v_stats.sent * 100);
    END IF;
    
    -- Determine new mode
    IF v_bounce_rate >= v_config.bounce_rate_critical 
       OR v_complaint_rate >= v_config.complaint_rate_critical THEN
      v_new_mode := 'critical';
      v_throttle := 0;
    ELSIF v_bounce_rate >= v_config.bounce_rate_warning 
          OR v_complaint_rate >= v_config.complaint_rate_warning THEN
      v_new_mode := 'warning';
      v_throttle := v_config.warning_throttle_percent;
    ELSE
      v_new_mode := 'normal';
      v_throttle := 100;
    END IF;
    
    -- Update state
    UPDATE sales_safe_mode_state SET
      mode = v_new_mode,
      current_bounce_rate = v_bounce_rate,
      current_complaint_rate = v_complaint_rate,
      emails_sent_7d = v_stats.sent,
      throttle_percent = v_throttle,
      mode_changed_at = CASE WHEN mode != v_new_mode THEN CURRENT_TIMESTAMP ELSE mode_changed_at END,
      mode_change_reason = CASE WHEN mode != v_new_mode THEN 
        format('Auto: bounce=%.1f%%, complaints=%.2f%%', v_bounce_rate, v_complaint_rate)
      ELSE mode_change_reason END,
      updated_at = CURRENT_TIMESTAMP
    WHERE id = 1;
  END;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- SUPPRESSION LIST VIEWS
-- ============================================================================

CREATE OR REPLACE VIEW suppression_list_summary AS
SELECT 
  suppression_type,
  COUNT(*) as count,
  COUNT(*) FILTER (WHERE expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP) as active_count,
  MAX(created_at) as latest_added
FROM sales_suppression_list
GROUP BY suppression_type
ORDER BY count DESC;

CREATE OR REPLACE VIEW recent_suppressions AS
SELECT 
  id,
  email,
  suppression_type,
  source_system,
  reason,
  created_at,
  created_by
FROM sales_suppression_list
ORDER BY created_at DESC
LIMIT 100;

-- ============================================================================
-- INDEXES FOR PERFORMANCE
-- ============================================================================

-- Fast safe mode lookups
CREATE INDEX IF NOT EXISTS idx_safe_mode_state_mode 
  ON sales_safe_mode_state(mode);

-- Comments
COMMENT ON TABLE sales_suppression_list IS 'Unified suppression list - emails that should NEVER receive outbound messages from any system';
COMMENT ON TABLE sales_safe_mode_config IS 'Configuration for automatic safe mode throttling';
COMMENT ON TABLE sales_safe_mode_state IS 'Current safe mode state and metrics';
COMMENT ON FUNCTION is_email_suppressed IS 'Check if an email is on the suppression list before sending';
COMMENT ON FUNCTION check_safe_mode IS 'Check current safe mode state and whether sending is allowed';
