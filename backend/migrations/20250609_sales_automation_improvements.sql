-- Migration: Add sales automation improvements tables
-- OVERCOMES competitor issues: spam, duplicates, unreliability, poor visibility
-- Based on research of Apollo, Instantly, Lemlist, Smartlead complaints

-- ===== Spam Prevention Tracking =====
-- Track spam checks and auto-fixes to maintain deliverability
CREATE TABLE IF NOT EXISTS spam_check_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID,
    contact_id UUID REFERENCES sales_contacts(id),
    checked_at TIMESTAMPTZ DEFAULT NOW(),
    original_score INTEGER NOT NULL,
    final_score INTEGER NOT NULL,
    triggers_found JSONB DEFAULT '[]',
    auto_fixes_applied JSONB DEFAULT '[]',
    passed BOOLEAN NOT NULL,
    subject_line TEXT,
    body_length INTEGER
);

CREATE INDEX IF NOT EXISTS idx_spam_checks_contact ON spam_check_logs(contact_id);
CREATE INDEX IF NOT EXISTS idx_spam_checks_passed ON spam_check_logs(passed, checked_at DESC);

-- ===== System Health Monitoring =====
-- Circuit breaker and health tracking for reliability
CREATE TABLE IF NOT EXISTS system_health_status (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    component VARCHAR(100) NOT NULL UNIQUE,
    status VARCHAR(20) NOT NULL DEFAULT 'healthy',
    last_check TIMESTAMPTZ DEFAULT NOW(),
    failures_count INTEGER DEFAULT 0,
    last_failure TIMESTAMPTZ,
    last_recovery TIMESTAMPTZ,
    circuit_state VARCHAR(20) DEFAULT 'closed',
    metadata JSONB DEFAULT '{}'
);

INSERT INTO system_health_status (component, status) VALUES 
    ('database', 'healthy'),
    ('email_service', 'healthy'),
    ('email_queue', 'healthy'),
    ('warmup_system', 'healthy'),
    ('bounce_rate', 'healthy')
ON CONFLICT (component) DO NOTHING;

CREATE TABLE IF NOT EXISTS recovery_actions_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    component VARCHAR(100) NOT NULL,
    action_type VARCHAR(100) NOT NULL,
    action_details JSONB,
    success BOOLEAN NOT NULL,
    error_message TEXT,
    executed_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_recovery_component ON recovery_actions_log(component, executed_at DESC);

-- ===== Deduplication Tracking =====
-- Track duplicate detection and merges
CREATE TABLE IF NOT EXISTS duplicate_detection_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    primary_contact_id UUID REFERENCES sales_contacts(id),
    duplicate_contact_id UUID,
    detection_method VARCHAR(50) NOT NULL,
    confidence DECIMAL(5,2) NOT NULL,
    merged BOOLEAN DEFAULT FALSE,
    merged_at TIMESTAMPTZ,
    details JSONB,
    detected_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_duplicates_primary ON duplicate_detection_log(primary_contact_id);
CREATE INDEX IF NOT EXISTS idx_duplicates_merged ON duplicate_detection_log(merged, detected_at DESC);

-- ===== Data Quality Tracking =====
-- Track data quality scores over time
CREATE TABLE IF NOT EXISTS data_quality_scores (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contact_id UUID REFERENCES sales_contacts(id),
    overall_score INTEGER NOT NULL,
    email_validity_score INTEGER,
    data_freshness_score INTEGER,
    completeness_score INTEGER,
    engagement_score INTEGER,
    bounce_history_score INTEGER,
    flags JSONB DEFAULT '[]',
    recommendations JSONB DEFAULT '[]',
    assessed_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quality_contact ON data_quality_scores(contact_id, assessed_at DESC);
CREATE INDEX IF NOT EXISTS idx_quality_score ON data_quality_scores(overall_score, assessed_at DESC);

-- Add quality-related columns to contacts
ALTER TABLE sales_contacts ADD COLUMN IF NOT EXISTS needs_reverification BOOLEAN DEFAULT FALSE;
ALTER TABLE sales_contacts ADD COLUMN IF NOT EXISTS last_quality_score INTEGER;
ALTER TABLE sales_contacts ADD COLUMN IF NOT EXISTS quality_flags TEXT[];

-- ===== Personalization Tracking =====
-- Track personalization effectiveness
CREATE TABLE IF NOT EXISTS personalization_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID,
    contact_id UUID REFERENCES sales_contacts(id),
    template_id UUID,
    personalization_score INTEGER NOT NULL,
    variations_used TEXT[],
    opening_type VARCHAR(50),
    closing_type VARCHAR(50),
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_personalization_contact ON personalization_log(contact_id);
CREATE INDEX IF NOT EXISTS idx_personalization_template ON personalization_log(template_id, created_at DESC);

-- ===== Automation Activity Logs =====
-- Comprehensive logging for transparency
CREATE TABLE IF NOT EXISTS automation_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    timestamp TIMESTAMPTZ DEFAULT NOW(),
    level VARCHAR(20) NOT NULL,
    category VARCHAR(50) NOT NULL,
    message TEXT NOT NULL,
    details JSONB,
    contact_id UUID REFERENCES sales_contacts(id),
    sequence_id UUID,
    email_id UUID,
    action_taken TEXT,
    resolution TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON automation_logs(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_logs_level ON automation_logs(level);
CREATE INDEX IF NOT EXISTS idx_logs_category ON automation_logs(category);
CREATE INDEX IF NOT EXISTS idx_logs_contact ON automation_logs(contact_id);

-- ===== System Health History =====
-- Historical health data for trend analysis
CREATE TABLE IF NOT EXISTS system_health_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    component VARCHAR(100) NOT NULL,
    status VARCHAR(20) NOT NULL,
    message TEXT,
    metrics JSONB,
    checked_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_health_component ON system_health_history(component, checked_at DESC);

-- ===== Email Send Window Optimization =====
-- Track best sending times per contact
CREATE TABLE IF NOT EXISTS send_time_analytics (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contact_id UUID REFERENCES sales_contacts(id),
    timezone VARCHAR(50),
    best_hour INTEGER,
    best_day VARCHAR(20),
    open_rate_by_hour JSONB DEFAULT '{}',
    click_rate_by_hour JSONB DEFAULT '{}',
    sample_size INTEGER DEFAULT 0,
    last_updated TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_send_time_contact ON send_time_analytics(contact_id);

-- ===== Bounce Prevention =====
-- Track email verification and deliverability
CREATE TABLE IF NOT EXISTS email_deliverability_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_address VARCHAR(255) NOT NULL,
    verified_at TIMESTAMPTZ,
    verification_method VARCHAR(50),
    is_deliverable BOOLEAN,
    risk_level VARCHAR(20),
    mx_records BOOLEAN,
    has_catch_all BOOLEAN,
    is_disposable BOOLEAN,
    is_role_based BOOLEAN,
    last_bounce_at TIMESTAMPTZ,
    bounce_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_deliverability_email ON email_deliverability_log(LOWER(email_address));
CREATE INDEX IF NOT EXISTS idx_deliverability_risk ON email_deliverability_log(risk_level, is_deliverable);

-- ===== Daily Stats Aggregation =====
-- Pre-computed stats for fast dashboards
CREATE TABLE IF NOT EXISTS daily_automation_stats (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    stat_date DATE NOT NULL UNIQUE,
    emails_sent INTEGER DEFAULT 0,
    emails_opened INTEGER DEFAULT 0,
    emails_clicked INTEGER DEFAULT 0,
    emails_replied INTEGER DEFAULT 0,
    emails_bounced INTEGER DEFAULT 0,
    contacts_discovered INTEGER DEFAULT 0,
    contacts_enriched INTEGER DEFAULT 0,
    sequences_started INTEGER DEFAULT 0,
    sequences_completed INTEGER DEFAULT 0,
    spam_checks_passed INTEGER DEFAULT 0,
    spam_checks_failed INTEGER DEFAULT 0,
    duplicates_found INTEGER DEFAULT 0,
    duplicates_merged INTEGER DEFAULT 0,
    system_errors INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_daily_stats_date ON daily_automation_stats(stat_date DESC);

-- ===== Function to update daily stats =====
CREATE OR REPLACE FUNCTION update_daily_automation_stats()
RETURNS void AS $$
DECLARE
    target_date DATE := CURRENT_DATE;
BEGIN
    INSERT INTO daily_automation_stats (stat_date)
    VALUES (target_date)
    ON CONFLICT (stat_date) DO NOTHING;
    
    UPDATE daily_automation_stats SET
        emails_sent = (SELECT COUNT(*) FROM outreach_emails WHERE DATE(sent_at) = target_date AND status = 'sent'),
        emails_opened = (SELECT COUNT(*) FROM outreach_emails WHERE DATE(opened_at) = target_date),
        emails_clicked = (SELECT COUNT(*) FROM outreach_emails WHERE DATE(clicked_at) = target_date),
        emails_replied = (SELECT COUNT(*) FROM outreach_emails WHERE DATE(replied_at) = target_date),
        emails_bounced = (SELECT COUNT(*) FROM outreach_emails WHERE DATE(bounced_at) = target_date),
        contacts_discovered = (SELECT COUNT(*) FROM sales_contacts WHERE DATE(created_at) = target_date),
        spam_checks_passed = (SELECT COUNT(*) FROM spam_check_logs WHERE DATE(checked_at) = target_date AND passed = true),
        spam_checks_failed = (SELECT COUNT(*) FROM spam_check_logs WHERE DATE(checked_at) = target_date AND passed = false),
        duplicates_found = (SELECT COUNT(*) FROM duplicate_detection_log WHERE DATE(detected_at) = target_date),
        duplicates_merged = (SELECT COUNT(*) FROM duplicate_detection_log WHERE DATE(merged_at) = target_date AND merged = true),
        system_errors = (SELECT COUNT(*) FROM automation_logs WHERE DATE(timestamp) = target_date AND level IN ('error', 'critical')),
        updated_at = NOW()
    WHERE stat_date = target_date;
END;
$$ LANGUAGE plpgsql;

COMMENT ON TABLE spam_check_logs IS 'Tracks spam prevention checks - OVERCOMES "emails landing in spam" complaint';
COMMENT ON TABLE system_health_status IS 'Circuit breaker state - OVERCOMES "system randomly stops" complaint';
COMMENT ON TABLE duplicate_detection_log IS 'Deduplication tracking - OVERCOMES "duplicates everywhere" complaint';
COMMENT ON TABLE data_quality_scores IS 'Data quality tracking - OVERCOMES "data is garbage" complaint';
COMMENT ON TABLE automation_logs IS 'Full activity logging - OVERCOMES "no visibility" complaint';
COMMENT ON TABLE personalization_log IS 'Personalization tracking - OVERCOMES "robotic templates" complaint';
