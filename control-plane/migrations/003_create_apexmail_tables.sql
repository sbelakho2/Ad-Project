-- ApexMail: Self-Hosted Email Infrastructure
-- Migration: 003_create_apexmail_tables.sql
-- Created: 2025-01-10
-- Description: Creates all tables for the ApexMail email system

BEGIN;

-- ============================================================================
-- Email Templates with Versioning
-- ============================================================================
CREATE TABLE IF NOT EXISTS apexmail_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    version INT NOT NULL DEFAULT 1,
    subject_template TEXT NOT NULL,
    body_html_template TEXT NOT NULL,
    body_text_template TEXT,
    variables JSONB DEFAULT '[]',  -- Required template variables
    category VARCHAR(50),  -- 'transactional', 'marketing', 'dunning'
    active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    CONSTRAINT apexmail_templates_name_version_unique UNIQUE (name, version)
);

CREATE INDEX IF NOT EXISTS idx_apexmail_templates_name ON apexmail_templates(name);
CREATE INDEX IF NOT EXISTS idx_apexmail_templates_category ON apexmail_templates(category);

-- ============================================================================
-- Email Queue with Priority and Retry Logic
-- ============================================================================
CREATE TABLE IF NOT EXISTS apexmail_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    
    -- Addressing
    from_email VARCHAR(255) NOT NULL,
    from_name VARCHAR(255),
    to_email VARCHAR(255) NOT NULL,
    reply_to VARCHAR(255),
    
    -- Content
    subject TEXT NOT NULL,
    body_html TEXT NOT NULL,
    body_text TEXT,
    headers JSONB DEFAULT '{}',
    
    -- Metadata
    template_id UUID REFERENCES apexmail_templates(id),
    template_data JSONB,
    category VARCHAR(50) NOT NULL DEFAULT 'transactional',
    priority INT DEFAULT 5,  -- 1=highest, 10=lowest
    
    -- Status tracking
    status VARCHAR(20) DEFAULT 'queued',
    attempt_count INT DEFAULT 0,
    max_attempts INT DEFAULT 3,
    next_attempt_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- SMTP response
    message_id VARCHAR(255),
    smtp_response TEXT,
    
    -- Timing
    created_at TIMESTAMPTZ DEFAULT NOW(),
    sent_at TIMESTAMPTZ,
    
    -- Reference tracking
    tracking_id VARCHAR(100) UNIQUE,
    entity_type VARCHAR(50),  -- 'contact', 'user', 'invoice'
    entity_id VARCHAR(100),
    
    -- Status constraint
    CONSTRAINT apexmail_queue_valid_status CHECK (
        status IN ('queued', 'sending', 'sent', 'failed', 'bounced', 'deferred')
    )
);

-- Indexes for queue processing
CREATE INDEX IF NOT EXISTS idx_apexmail_queue_next ON apexmail_queue(next_attempt_at) 
    WHERE status IN ('queued', 'deferred');
CREATE INDEX IF NOT EXISTS idx_apexmail_queue_priority ON apexmail_queue(priority, created_at) 
    WHERE status = 'queued';
CREATE INDEX IF NOT EXISTS idx_apexmail_queue_status ON apexmail_queue(status);
CREATE INDEX IF NOT EXISTS idx_apexmail_queue_tracking ON apexmail_queue(tracking_id);
CREATE INDEX IF NOT EXISTS idx_apexmail_queue_entity ON apexmail_queue(entity_type, entity_id);

-- ============================================================================
-- Comprehensive Event Tracking
-- ============================================================================
CREATE TABLE IF NOT EXISTS apexmail_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID REFERENCES apexmail_queue(id) ON DELETE SET NULL,
    tracking_id VARCHAR(100),
    
    event_type VARCHAR(30) NOT NULL,  -- sent, delivered, opened, clicked, bounced, complained, unsubscribed
    
    -- Event metadata
    ip_address INET,
    user_agent TEXT,
    link_url TEXT,  -- For click events
    bounce_type VARCHAR(30),  -- hard, soft, complaint
    bounce_reason TEXT,
    
    -- Geo data (derived from IP)
    country_code VARCHAR(2),
    city VARCHAR(100),
    
    -- Timing
    occurred_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- Raw webhook data
    raw_data JSONB
);

CREATE INDEX IF NOT EXISTS idx_apexmail_events_tracking ON apexmail_events(tracking_id);
CREATE INDEX IF NOT EXISTS idx_apexmail_events_type ON apexmail_events(event_type, occurred_at);
CREATE INDEX IF NOT EXISTS idx_apexmail_events_email ON apexmail_events(email_id);

-- ============================================================================
-- Domain Health Monitoring (Anti-Spam Protection)
-- ============================================================================
CREATE TABLE IF NOT EXISTS apexmail_domain_health (
    domain VARCHAR(255) PRIMARY KEY,
    
    -- Rolling 7-day metrics
    sent_7d INT DEFAULT 0,
    delivered_7d INT DEFAULT 0,
    bounced_7d INT DEFAULT 0,
    complained_7d INT DEFAULT 0,
    
    -- Calculated rates
    bounce_rate_7d DECIMAL(5,4) DEFAULT 0,
    complaint_rate_7d DECIMAL(5,4) DEFAULT 0,
    
    -- Status
    status VARCHAR(20) DEFAULT 'healthy',  -- healthy, warning, critical, suspended
    suspended_at TIMESTAMPTZ,
    suspension_reason TEXT,
    
    -- Last update
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- ============================================================================
-- Suppression List (Never Send to These)
-- ============================================================================
CREATE TABLE IF NOT EXISTS apexmail_suppressions (
    email VARCHAR(255) PRIMARY KEY,
    reason VARCHAR(30) NOT NULL,  -- hard_bounce, complaint, unsubscribe, manual
    source VARCHAR(100),
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_apexmail_suppressions_reason ON apexmail_suppressions(reason);

-- ============================================================================
-- Link Tracking for Click Analytics
-- ============================================================================
CREATE TABLE IF NOT EXISTS apexmail_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID REFERENCES apexmail_queue(id) ON DELETE SET NULL,
    tracking_id VARCHAR(100),
    original_url TEXT NOT NULL,
    short_code VARCHAR(20) UNIQUE,
    click_count INT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_apexmail_links_code ON apexmail_links(short_code);
CREATE INDEX IF NOT EXISTS idx_apexmail_links_tracking ON apexmail_links(tracking_id);

-- ============================================================================
-- Seed Default Templates
-- ============================================================================
INSERT INTO apexmail_templates (name, subject_template, body_html_template, body_text_template, category, variables) VALUES
(
    'email-verification',
    'Verify your email address',
    '<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto; padding: 20px;">
  <h1 style="color: #333;">Welcome to ApexMediation!</h1>
  <p>Please verify your email address by clicking the button below:</p>
  <p style="text-align: center; margin: 30px 0;">
    <a href="{{verificationUrl}}" style="background-color: #007bff; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">Verify Email</a>
  </p>
  <p style="color: #666; font-size: 14px;">If you didn''t create an account, you can safely ignore this email.</p>
</body>
</html>',
    'Welcome to ApexMediation!

Please verify your email address by clicking this link:
{{verificationUrl}}

If you didn''t create an account, you can safely ignore this email.',
    'transactional',
    '["verificationUrl"]'
),
(
    'password-reset',
    'Reset your password',
    '<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto; padding: 20px;">
  <h1 style="color: #333;">Password Reset Request</h1>
  <p>We received a request to reset your password. Click the button below to proceed:</p>
  <p style="text-align: center; margin: 30px 0;">
    <a href="{{resetUrl}}" style="background-color: #dc3545; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">Reset Password</a>
  </p>
  <p style="color: #666; font-size: 14px;">This link will expire in 1 hour. If you didn''t request this, you can safely ignore this email.</p>
</body>
</html>',
    'Password Reset Request

We received a request to reset your password. Click this link to proceed:
{{resetUrl}}

This link will expire in 1 hour. If you didn''t request this, you can safely ignore this email.',
    'transactional',
    '["resetUrl"]'
),
(
    'payment-failed',
    'Payment failed for your subscription',
    '<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto; padding: 20px;">
  <h1 style="color: #dc3545;">Payment Failed</h1>
  <p>We were unable to process your payment of <strong>{{amount}} {{currency}}</strong>.</p>
  <p>Please update your payment method to continue your subscription:</p>
  <p style="text-align: center; margin: 30px 0;">
    <a href="{{paymentUrl}}" style="background-color: #28a745; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">Update Payment Method</a>
  </p>
  <p style="color: #666; font-size: 14px;">If you have any questions, please contact our support team.</p>
</body>
</html>',
    'Payment Failed

We were unable to process your payment of {{amount}} {{currency}}.

Please update your payment method to continue your subscription:
{{paymentUrl}}

If you have any questions, please contact our support team.',
    'dunning',
    '["amount", "currency", "paymentUrl"]'
),
(
    'device-approval',
    'New device login - Action Required',
    '<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto; padding: 20px;">
  <h1 style="color: #333;">New Device Login Request</h1>
  <p>A login attempt was made from a new device:</p>
  <ul style="background: #f5f5f5; padding: 15px 30px; border-radius: 4px;">
    <li><strong>Device:</strong> {{deviceInfo}}</li>
    <li><strong>IP Address:</strong> {{ipAddress}}</li>
    <li><strong>Time:</strong> {{timestamp}}</li>
  </ul>
  <p>If this was you, click below to approve this device:</p>
  <p style="text-align: center; margin: 30px 0;">
    <a href="{{approveUrl}}" style="background-color: #28a745; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">Approve Device</a>
  </p>
  <p style="color: #dc3545; font-size: 14px;"><strong>If this wasn''t you</strong>, your account may be compromised. Please change your password immediately.</p>
</body>
</html>',
    'New Device Login Request

A login attempt was made from a new device:
- Device: {{deviceInfo}}
- IP Address: {{ipAddress}}
- Time: {{timestamp}}

If this was you, click this link to approve:
{{approveUrl}}

If this wasn''t you, your account may be compromised. Please change your password immediately.',
    'transactional',
    '["deviceInfo", "ipAddress", "timestamp", "approveUrl"]'
),
(
    'sales-outreach',
    '{{subject}}',
    '{{body}}',
    '{{textBody}}',
    'marketing',
    '["subject", "body", "textBody"]'
);

COMMIT;
