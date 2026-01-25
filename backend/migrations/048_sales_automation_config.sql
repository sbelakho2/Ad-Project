-- Migration: Sales Automation Configuration Table
-- Enables full control of sender config, review queues, and verification settings from UI

-- ============================================================================
-- CONFIG TABLE (replaces hardcoded values)
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_config (
  key VARCHAR(100) PRIMARY KEY,
  value TEXT NOT NULL,
  description TEXT,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_by VARCHAR(255)
);

-- Insert default configuration values
INSERT INTO sa_config (key, value, description) VALUES
  ('from_email', 'sales@apexmediation.ee', 'Default sender email address'),
  ('from_name', 'Aaron from ApexMediation', 'Default sender display name'),
  ('reply_to', 'sales@apexmediation.ee', 'Reply-to email address'),
  ('daily_limit', '100', 'Maximum emails to send per day'),
  ('require_email_verification', 'false', 'Require email verification before enrolling contacts'),
  ('enable_review_queue', 'false', 'Create drafts instead of sending immediately'),
  ('warmup_mode', 'false', 'Enable gradual sending increase for new domains'),
  ('warmup_daily_increment', '5', 'Number of additional emails per day during warmup'),
  ('system_enabled', 'false', 'Master switch for sales automation'),
  ('spam_score_threshold', '70', 'Minimum spam score (0-100) to allow sending'),
  ('bounce_rate_threshold', '7', 'Maximum bounce rate (%) before auto-pause'),
  ('pause_on_critical_health', 'true', 'Automatically pause when domain health is critical')
ON CONFLICT (key) DO NOTHING;

-- ============================================================================
-- ADD DRAFT STATE TO EMAILS
-- ============================================================================

-- Add 'draft' status to sa_emails if not exists
-- Existing statuses: pending, sent, opened, clicked, replied, bounced
-- New status: draft (for review queue)
-- Flow: draft -> pending -> sent

ALTER TABLE sa_emails ADD COLUMN IF NOT EXISTS approved_at TIMESTAMP;
ALTER TABLE sa_emails ADD COLUMN IF NOT EXISTS approved_by VARCHAR(255);
ALTER TABLE sa_emails ADD COLUMN IF NOT EXISTS rejection_reason TEXT;

-- ============================================================================
-- ADD VERIFICATION FIELDS TO CONTACTS
-- ============================================================================

ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS email_verified BOOLEAN DEFAULT false;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS email_verification_status VARCHAR(30) DEFAULT 'unverified';
-- Statuses: unverified, pending, valid, invalid, catch_all, risky

-- Index for finding unverified contacts
CREATE INDEX IF NOT EXISTS idx_sa_contacts_verified ON sa_contacts(email_verified);

-- ============================================================================
-- HELPER FUNCTION TO GET CONFIG VALUE
-- ============================================================================

CREATE OR REPLACE FUNCTION sa_get_config(p_key VARCHAR, p_default TEXT DEFAULT NULL)
RETURNS TEXT AS $$
DECLARE
  v_value TEXT;
BEGIN
  SELECT value INTO v_value FROM sa_config WHERE key = p_key;
  RETURN COALESCE(v_value, p_default);
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- COMMENTS
-- ============================================================================

COMMENT ON TABLE sa_config IS 'Sales Automation configuration - editable from Control Plane UI';
COMMENT ON COLUMN sa_emails.approved_at IS 'When the draft was approved for sending';
COMMENT ON COLUMN sa_emails.approved_by IS 'Who approved the email (operator email)';
COMMENT ON COLUMN sa_contacts.email_verified IS 'Whether the email has been verified';
