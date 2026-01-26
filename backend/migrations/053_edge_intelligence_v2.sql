-- Migration: Edge Intelligence V2 (On-Device ML Features)
-- 
-- Adds tables for:
--   1. Icebreaker cache (LLM-generated personalization)
--   2. Bounce records (DMARC/SPF reputation monitoring)
--   3. Reputation alerts
--   4. Hybrid classification audit log
--
-- Run on: App Server Database (via Control Plane)
-- Date: 2026-01-25

BEGIN;

-- ============================================================================
-- 1. ICEBREAKER CACHE
-- ============================================================================
-- Stores LLM-generated icebreakers to avoid re-computation
-- TTL: 30 days (handled at query time)

CREATE TABLE IF NOT EXISTS sa_icebreaker_cache (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_name TEXT NOT NULL UNIQUE,
    icebreaker TEXT NOT NULL,
    source TEXT DEFAULT 'llm' CHECK (source IN ('llm', 'template')),
    model_used TEXT,
    latency_ms INTEGER,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_icebreaker_cache_app_name ON sa_icebreaker_cache(app_name);
CREATE INDEX IF NOT EXISTS idx_sa_icebreaker_cache_created_at ON sa_icebreaker_cache(created_at);

-- ============================================================================
-- 2. BOUNCE RECORDS (Reputation Monitoring)
-- ============================================================================
-- Stores detailed bounce analysis for reputation tracking

CREATE TABLE IF NOT EXISTS sa_bounce_records (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id TEXT,
    contact_id UUID REFERENCES sa_contacts(id) ON DELETE SET NULL,
    bounce_type TEXT NOT NULL CHECK (bounce_type IN ('hard', 'soft', 'policy')),
    error_code TEXT,
    error_message TEXT,
    is_dmarc_failure BOOLEAN DEFAULT FALSE,
    is_spf_failure BOOLEAN DEFAULT FALSE,
    is_dkim_failure BOOLEAN DEFAULT FALSE,
    is_reputation_issue BOOLEAN DEFAULT FALSE,
    risk_level TEXT DEFAULT 'LOW' CHECK (risk_level IN ('LOW', 'MEDIUM', 'HIGH', 'CRITICAL')),
    raw_diagnostic TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_bounce_records_created_at ON sa_bounce_records(created_at);
CREATE INDEX IF NOT EXISTS idx_sa_bounce_records_bounce_type ON sa_bounce_records(bounce_type);
CREATE INDEX IF NOT EXISTS idx_sa_bounce_records_risk_level ON sa_bounce_records(risk_level);
CREATE INDEX IF NOT EXISTS idx_sa_bounce_records_contact_id ON sa_bounce_records(contact_id);

-- ============================================================================
-- 3. REPUTATION ALERTS
-- ============================================================================
-- Stores reputation-related alerts for operator review

CREATE TABLE IF NOT EXISTS sa_reputation_alerts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'critical')),
    type TEXT NOT NULL,
    message TEXT NOT NULL,
    details JSONB DEFAULT '{}',
    acknowledged BOOLEAN DEFAULT FALSE,
    acknowledged_by TEXT,
    acknowledged_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_reputation_alerts_created_at ON sa_reputation_alerts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_sa_reputation_alerts_severity ON sa_reputation_alerts(severity);
CREATE INDEX IF NOT EXISTS idx_sa_reputation_alerts_acknowledged ON sa_reputation_alerts(acknowledged);

-- ============================================================================
-- 4. HYBRID CLASSIFICATION AUDIT LOG
-- ============================================================================
-- Tracks hybrid classifier decisions for debugging and improvement

CREATE TABLE IF NOT EXISTS sa_classification_audit (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    inbox_message_id UUID REFERENCES sa_inbox_messages(id) ON DELETE CASCADE,
    fast_pass_category TEXT NOT NULL,
    fast_pass_confidence NUMERIC(5,4) NOT NULL,
    deep_pass_category TEXT,
    deep_pass_confidence NUMERIC(5,4),
    final_category TEXT NOT NULL,
    final_confidence NUMERIC(5,4) NOT NULL,
    consensus_reached BOOLEAN NOT NULL,
    processing_mode TEXT NOT NULL CHECK (processing_mode IN ('fast', 'deep', 'hybrid')),
    latency_ms_fast INTEGER NOT NULL,
    latency_ms_deep INTEGER,
    latency_ms_total INTEGER NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_classification_audit_created_at ON sa_classification_audit(created_at);
CREATE INDEX IF NOT EXISTS idx_sa_classification_audit_consensus ON sa_classification_audit(consensus_reached);
CREATE INDEX IF NOT EXISTS idx_sa_classification_audit_mode ON sa_classification_audit(processing_mode);

-- ============================================================================
-- 5. ADD ICEBREAKER COLUMN TO CONTACTS (if not exists)
-- ============================================================================

DO $$ 
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'sa_contacts' AND column_name = 'icebreaker'
    ) THEN
        ALTER TABLE sa_contacts ADD COLUMN icebreaker TEXT;
    END IF;
END $$;

-- ============================================================================
-- 6. ADD APP DESCRIPTION COLUMN TO CONTACTS (if not exists)
-- ============================================================================

DO $$ 
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'sa_contacts' AND column_name = 'app_description'
    ) THEN
        ALTER TABLE sa_contacts ADD COLUMN app_description TEXT;
    END IF;
END $$;

-- ============================================================================
-- 7. METRICS VIEW FOR REPUTATION DASHBOARD
-- ============================================================================

CREATE OR REPLACE VIEW v_sa_reputation_metrics AS
SELECT 
    'day' AS period,
    COUNT(DISTINCT om.id) AS total_sent,
    COUNT(DISTINCT br.id) AS total_bounced,
    COUNT(DISTINCT br.id) FILTER (WHERE br.bounce_type = 'hard') AS hard_bounces,
    COUNT(DISTINCT br.id) FILTER (WHERE br.bounce_type = 'soft') AS soft_bounces,
    COUNT(DISTINCT br.id) FILTER (WHERE br.bounce_type = 'policy') AS policy_bounces,
    COUNT(DISTINCT br.id) FILTER (WHERE br.is_dmarc_failure = true) AS dmarc_failures,
    COUNT(DISTINCT br.id) FILTER (WHERE br.is_spf_failure = true) AS spf_failures,
    COUNT(DISTINCT br.id) FILTER (WHERE br.is_dkim_failure = true) AS dkim_failures,
    CASE 
        WHEN COUNT(DISTINCT om.id) > 0 
        THEN ROUND(COUNT(DISTINCT br.id)::NUMERIC / COUNT(DISTINCT om.id), 4)
        ELSE 0 
    END AS bounce_rate,
    CURRENT_TIMESTAMP AS calculated_at
FROM sa_outbound_messages om
LEFT JOIN sa_bounce_records br ON br.created_at > NOW() - INTERVAL '24 hours'
WHERE om.created_at > NOW() - INTERVAL '24 hours';

COMMIT;
