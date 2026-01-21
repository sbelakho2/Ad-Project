-- Migration: 20250610_revolutionary_sales_features.sql
-- Revolutionary sales automation features - fully autonomous system
-- Based on deep competitive analysis and Reddit r/coldemail insights

-- ============================================
-- 1. PREDICTIVE LEAD SCORING
-- Revolutionary: Score contacts BEFORE outreach
-- ============================================

CREATE TABLE IF NOT EXISTS lead_scores (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contact_id UUID REFERENCES sales_contacts(id) ON DELETE CASCADE,
    score INTEGER NOT NULL DEFAULT 0,
    priority_tier VARCHAR(20) DEFAULT 'cold',
    signals JSONB DEFAULT '{}',
    engagement_velocity FLOAT DEFAULT 0,
    predicted_best_time TIMESTAMPTZ,
    calculated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(contact_id)
);

CREATE INDEX IF NOT EXISTS idx_lead_scores_score ON lead_scores(score DESC);
CREATE INDEX IF NOT EXISTS idx_lead_scores_tier ON lead_scores(priority_tier, score DESC);
CREATE INDEX IF NOT EXISTS idx_lead_scores_velocity ON lead_scores(engagement_velocity DESC);

-- ============================================
-- 2. SELF-LEARNING SEQUENCES
-- Revolutionary: Sequences that optimize themselves
-- ============================================

CREATE TABLE IF NOT EXISTS sequence_ab_tests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence_id UUID REFERENCES outreach_sequences(id) ON DELETE CASCADE,
    step_number INTEGER NOT NULL,
    variant_a_subject TEXT,
    variant_a_body TEXT,
    variant_b_subject TEXT,
    variant_b_body TEXT,
    variant_a_sent INTEGER DEFAULT 0,
    variant_a_opens INTEGER DEFAULT 0,
    variant_a_clicks INTEGER DEFAULT 0,
    variant_a_replies INTEGER DEFAULT 0,
    variant_b_sent INTEGER DEFAULT 0,
    variant_b_opens INTEGER DEFAULT 0,
    variant_b_clicks INTEGER DEFAULT 0,
    variant_b_replies INTEGER DEFAULT 0,
    status VARCHAR(20) DEFAULT 'running',
    winner VARCHAR(10),
    confidence FLOAT,
    started_at TIMESTAMPTZ DEFAULT NOW(),
    ended_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_ab_tests_sequence ON sequence_ab_tests(sequence_id, step_number);
CREATE INDEX IF NOT EXISTS idx_ab_tests_status ON sequence_ab_tests(status);

CREATE TABLE IF NOT EXISTS template_suggestions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence_id UUID REFERENCES outreach_sequences(id) ON DELETE CASCADE,
    step_number INTEGER,
    suggestion_type VARCHAR(50),
    original_text TEXT,
    suggested_text TEXT,
    reason TEXT,
    expected_improvement FLOAT,
    applied BOOLEAN DEFAULT FALSE,
    applied_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS sequence_performance_patterns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pattern_type VARCHAR(50),
    pattern_description TEXT,
    affected_metric VARCHAR(50),
    impact_percentage FLOAT,
    sample_size INTEGER,
    confidence FLOAT,
    discovered_at TIMESTAMPTZ DEFAULT NOW()
);

-- ============================================
-- 3. SMART REPLY ANALYZER
-- Revolutionary: Instant categorization for fast response
-- ============================================

CREATE TABLE IF NOT EXISTS reply_analysis (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID,
    contact_id UUID REFERENCES sales_contacts(id) ON DELETE CASCADE,
    category VARCHAR(50) NOT NULL,
    sentiment VARCHAR(20) NOT NULL,
    urgency VARCHAR(20) NOT NULL,
    confidence INTEGER NOT NULL,
    suggested_action TEXT,
    key_phrases TEXT[],
    response_deadline TIMESTAMPTZ,
    analyzed_at TIMESTAMPTZ DEFAULT NOW(),
    responded_at TIMESTAMPTZ,
    response_time_minutes INTEGER
);

CREATE INDEX IF NOT EXISTS idx_reply_analysis_contact ON reply_analysis(contact_id);
CREATE INDEX IF NOT EXISTS idx_reply_analysis_category ON reply_analysis(category, analyzed_at DESC);
CREATE INDEX IF NOT EXISTS idx_reply_analysis_urgency ON reply_analysis(urgency, responded_at);
CREATE INDEX IF NOT EXISTS idx_reply_analysis_pending ON reply_analysis(category, urgency) WHERE responded_at IS NULL;

-- ============================================
-- 4. DOMAIN HEALTH GUARD
-- Revolutionary: Proactive reputation protection
-- ============================================

CREATE TABLE IF NOT EXISTS domain_health (
    domain VARCHAR(255) PRIMARY KEY,
    mailboxes TEXT[],
    health_score INTEGER DEFAULT 100,
    status VARCHAR(50) DEFAULT 'healthy',
    first_send_date TIMESTAMPTZ,
    warmup_complete BOOLEAN DEFAULT FALSE,
    quarantined_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS domain_health_alerts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    domain VARCHAR(255) REFERENCES domain_health(domain) ON DELETE CASCADE,
    alert_type VARCHAR(50) NOT NULL,
    severity VARCHAR(20) NOT NULL,
    message TEXT,
    triggered_at TIMESTAMPTZ DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    resolved BOOLEAN DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_domain_alerts ON domain_health_alerts(domain, resolved);
CREATE INDEX IF NOT EXISTS idx_domain_alerts_severity ON domain_health_alerts(severity, resolved);

CREATE TABLE IF NOT EXISTS domain_daily_stats (
    domain VARCHAR(255),
    date DATE,
    emails_sent INTEGER DEFAULT 0,
    bounces INTEGER DEFAULT 0,
    spam_reports INTEGER DEFAULT 0,
    opens INTEGER DEFAULT 0,
    replies INTEGER DEFAULT 0,
    unsubscribes INTEGER DEFAULT 0,
    PRIMARY KEY (domain, date)
);

CREATE INDEX IF NOT EXISTS idx_domain_stats_date ON domain_daily_stats(date DESC);

-- ============================================
-- 5. INTENT SIGNAL DETECTOR
-- Revolutionary: Know who's ready to buy
-- ============================================

CREATE TABLE IF NOT EXISTS intent_signals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contact_id UUID REFERENCES sales_contacts(id) ON DELETE CASCADE,
    signal_type VARCHAR(50) NOT NULL,
    strength INTEGER NOT NULL DEFAULT 5,
    source VARCHAR(100),
    details TEXT,
    detected_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_intent_signals_contact ON intent_signals(contact_id, detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_intent_signals_type ON intent_signals(signal_type, detected_at DESC);

CREATE TABLE IF NOT EXISTS intent_scores (
    contact_id UUID PRIMARY KEY REFERENCES sales_contacts(id) ON DELETE CASCADE,
    intent_score INTEGER NOT NULL DEFAULT 0,
    intent_level VARCHAR(20) NOT NULL DEFAULT 'cold',
    buying_stage VARCHAR(20) NOT NULL DEFAULT 'unknown',
    last_signal_at TIMESTAMPTZ,
    calculated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_intent_scores_level ON intent_scores(intent_level, intent_score DESC);
CREATE INDEX IF NOT EXISTS idx_intent_scores_stage ON intent_scores(buying_stage);

-- ============================================
-- 6. FOLLOW-UP OPTIMIZER
-- Revolutionary: Follow-ups as main event
-- ============================================

CREATE TABLE IF NOT EXISTS followup_analytics (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence_id UUID REFERENCES outreach_sequences(id) ON DELETE CASCADE,
    step_number INTEGER NOT NULL,
    period_start DATE NOT NULL,
    period_end DATE NOT NULL,
    emails_sent INTEGER DEFAULT 0,
    opens INTEGER DEFAULT 0,
    clicks INTEGER DEFAULT 0,
    replies INTEGER DEFAULT 0,
    positive_replies INTEGER DEFAULT 0,
    conversions INTEGER DEFAULT 0,
    avg_response_time_hours FLOAT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_followup_analytics_seq ON followup_analytics(sequence_id, step_number);

CREATE TABLE IF NOT EXISTS followup_recommendations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence_id UUID REFERENCES outreach_sequences(id) ON DELETE CASCADE,
    step_number INTEGER NOT NULL,
    recommendation_type VARCHAR(50),
    recommendation TEXT,
    potential_lift FLOAT,
    applied BOOLEAN DEFAULT FALSE,
    applied_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_followup_recommendations_seq ON followup_recommendations(sequence_id, applied);

-- ============================================
-- 7. AUTOMATION ENGINE TRACKING
-- Track the master orchestrator
-- ============================================

CREATE TABLE IF NOT EXISTS automation_cycles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    started_at TIMESTAMPTZ DEFAULT NOW(),
    ended_at TIMESTAMPTZ,
    status VARCHAR(20) DEFAULT 'running',
    tasks_completed INTEGER DEFAULT 0,
    tasks_failed INTEGER DEFAULT 0,
    emails_sent INTEGER DEFAULT 0,
    errors JSONB DEFAULT '[]',
    summary JSONB DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_automation_cycles_date ON automation_cycles(started_at DESC);

CREATE TABLE IF NOT EXISTS reliability_errors (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    error_type VARCHAR(50),
    error_message TEXT,
    stack_trace TEXT,
    context JSONB,
    occurred_at TIMESTAMPTZ DEFAULT NOW(),
    resolved BOOLEAN DEFAULT FALSE,
    resolved_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_reliability_errors_date ON reliability_errors(occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_reliability_errors_type ON reliability_errors(error_type, resolved);

-- ============================================
-- 8. VIEWS FOR EASY REPORTING
-- ============================================

-- Hot leads view
CREATE OR REPLACE VIEW v_hot_leads AS
SELECT 
    c.id,
    c.email,
    c.first_name,
    c.last_name,
    c.company_name,
    ls.score as lead_score,
    ls.priority_tier,
    is2.intent_score,
    is2.intent_level,
    is2.buying_stage,
    ls.predicted_best_time
FROM sales_contacts c
LEFT JOIN lead_scores ls ON c.id = ls.contact_id
LEFT JOIN intent_scores is2 ON c.id = is2.contact_id
WHERE ls.priority_tier IN ('hot', 'warm')
   OR is2.intent_level IN ('hot', 'warm')
ORDER BY COALESCE(ls.score, 0) + COALESCE(is2.intent_score, 0) DESC;

-- Domain health dashboard view
CREATE OR REPLACE VIEW v_domain_health_dashboard AS
SELECT 
    dh.domain,
    dh.health_score,
    dh.status,
    array_length(dh.mailboxes, 1) as mailbox_count,
    dh.warmup_complete,
    dds.emails_sent as today_sent,
    dds.bounces as today_bounces,
    dds.spam_reports as today_spam,
    CASE WHEN dds.emails_sent > 0 
         THEN (dds.bounces::float / dds.emails_sent * 100)::numeric(5,2)
         ELSE 0 
    END as bounce_rate,
    (SELECT COUNT(*) FROM domain_health_alerts 
     WHERE domain = dh.domain AND resolved = FALSE) as active_alerts
FROM domain_health dh
LEFT JOIN domain_daily_stats dds ON dh.domain = dds.domain 
    AND dds.date = CURRENT_DATE;

-- Pending replies view
CREATE OR REPLACE VIEW v_pending_replies AS
SELECT 
    ra.id,
    c.email,
    c.first_name || ' ' || c.last_name as contact_name,
    ra.category,
    ra.sentiment,
    ra.urgency,
    ra.suggested_action,
    ra.response_deadline,
    EXTRACT(EPOCH FROM (NOW() - ra.analyzed_at))/60 as waiting_minutes
FROM reply_analysis ra
JOIN sales_contacts c ON ra.contact_id = c.id
WHERE ra.responded_at IS NULL
ORDER BY 
    CASE ra.urgency 
        WHEN 'high' THEN 1 
        WHEN 'medium' THEN 2 
        ELSE 3 
    END,
    ra.analyzed_at ASC;

-- Sequence performance view
CREATE OR REPLACE VIEW v_sequence_performance AS
SELECT 
    os.id as sequence_id,
    os.name,
    ss.step_number,
    COUNT(oe.id) as total_sent,
    COUNT(oe.opened_at) as opens,
    COUNT(oe.clicked_at) as clicks,
    COUNT(oe.replied_at) as replies,
    CASE WHEN COUNT(oe.id) > 0 
         THEN (COUNT(oe.replied_at)::float / COUNT(oe.id) * 100)::numeric(5,2)
         ELSE 0 
    END as reply_rate
FROM outreach_sequences os
JOIN sequence_steps ss ON os.id = ss.sequence_id
LEFT JOIN outreach_emails oe ON ss.sequence_id = oe.sequence_id 
    AND ss.step_number = oe.step_number
    AND oe.sent_at > NOW() - INTERVAL '30 days'
GROUP BY os.id, os.name, ss.step_number
ORDER BY os.name, ss.step_number;

-- ============================================
-- 9. HELPFUL FUNCTIONS
-- ============================================

-- Function to get contact's engagement score quickly
CREATE OR REPLACE FUNCTION get_engagement_score(p_contact_id UUID)
RETURNS INTEGER AS $$
DECLARE
    v_score INTEGER := 0;
BEGIN
    SELECT COALESCE(
        (SELECT score FROM lead_scores WHERE contact_id = p_contact_id) +
        (SELECT intent_score FROM intent_scores WHERE contact_id = p_contact_id),
        0
    ) INTO v_score;
    
    RETURN v_score;
END;
$$ LANGUAGE plpgsql;

-- Function to check if safe to send
CREATE OR REPLACE FUNCTION can_send_from_domain(p_domain VARCHAR)
RETURNS BOOLEAN AS $$
DECLARE
    v_health domain_health%ROWTYPE;
    v_today_sent INTEGER;
BEGIN
    SELECT * INTO v_health FROM domain_health WHERE domain = p_domain;
    
    -- If quarantined, can't send
    IF v_health.quarantined_until IS NOT NULL AND v_health.quarantined_until > NOW() THEN
        RETURN FALSE;
    END IF;
    
    -- If critical status, can't send
    IF v_health.status = 'critical' THEN
        RETURN FALSE;
    END IF;
    
    -- Check daily limit
    SELECT COALESCE(emails_sent, 0) INTO v_today_sent
    FROM domain_daily_stats 
    WHERE domain = p_domain AND date = CURRENT_DATE;
    
    -- During warmup, limit is 20
    IF NOT v_health.warmup_complete AND v_today_sent >= 20 THEN
        RETURN FALSE;
    END IF;
    
    -- After warmup, limit is 100
    IF v_today_sent >= 100 THEN
        RETURN FALSE;
    END IF;
    
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql;

-- ============================================
-- 10. GRANT PERMISSIONS
-- ============================================

-- If using a specific app user, grant permissions
-- GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO app_user;
-- GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO app_user;

COMMENT ON TABLE lead_scores IS 'Revolutionary predictive lead scoring - score contacts BEFORE outreach';
COMMENT ON TABLE sequence_ab_tests IS 'Self-learning A/B tests - sequences optimize themselves';
COMMENT ON TABLE reply_analysis IS 'Smart reply categorization for instant prioritization';
COMMENT ON TABLE domain_health IS 'Proactive domain reputation protection';
COMMENT ON TABLE intent_signals IS 'Buying intent detection from engagement signals';
COMMENT ON TABLE followup_analytics IS 'Follow-up optimization - treating follow-ups as main event';
