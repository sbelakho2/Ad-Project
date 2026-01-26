-- ============================================================================
-- Extended Thompson Sampler Arms v2 (January 2026)
-- Purpose: Add more diverse subject line variants for better A/B testing
-- Based on: Analysis report recommendations + Cialdini psychology
-- 
-- KEY INSIGHT: Need 5+ active arms for meaningful Thompson sampling
-- Previous set had 12, but many may have been retired. This ensures 
-- we always have enough active arms for effective optimization.
-- ============================================================================

BEGIN;

-- ============================================================================
-- ADDITIONAL PSYCHOLOGY-BASED SUBJECT LINE ARMS
-- ============================================================================
-- These complement the existing arms with different psychological triggers

-- ARM 13: Loss Aversion (Leaving money on table)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'loss_aversion_money', 'Is {{APP_NAME}} leaving ad revenue on the table?', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 14: Specificity (Exact numbers build trust)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'specificity_percentage', '{{APP_NAME}}: Potential 23% fill rate improvement', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 15: Simplicity (Plain language crushes cleverness)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'simplicity_plain', 'Saw {{APP_NAME}} - had a thought on ad revenue', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 16: Curiosity + Personalization
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'curiosity_noticed', 'Noticed something in {{APP_NAME}}''s ad setup', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 17: Social Proof + Competitor Angle
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'competitor_switch', 'Why {{APP_GENRE}} devs are switching from IronSource', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 18: Short & Direct (Mobile-optimized, under 35 chars)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'mobile_short', '{{APP_NAME}} ad revenue', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 19: Question Format (Drives engagement)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'question_direct', 'Using mediation for {{APP_NAME}}?', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ARM 20: Exclusivity (FOMO trigger)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'exclusivity_invite', 'Exclusive: {{APP_GENRE}} publisher insights', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- FOLLOW-UP SUBJECT LINE ARMS (For sequences)
-- ============================================================================

-- Follow-up ARM 1: Re-engagement
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('followup_subject', 'followup_bump', 'Re: {{APP_NAME}} revenue analysis', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- Follow-up ARM 2: Value reminder
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('followup_subject', 'followup_value', 'Quick update on your {{APP_NAME}} audit', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- Follow-up ARM 3: Social proof
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('followup_subject', 'followup_social', 'A {{APP_GENRE}} publisher just saw 28% lift', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- Follow-up ARM 4: Final touch (scarcity)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('followup_subject', 'followup_final', 'Last note about {{APP_NAME}}', 1, 1, 50)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- VERIFICATION QUERIES
-- ============================================================================

-- Count active arms by type
SELECT arm_type, COUNT(*) as active_count
FROM sa_bandit_arms 
WHERE active = true
GROUP BY arm_type
ORDER BY arm_type;

-- Show all active subject line arms
SELECT id, arm_name, LEFT(arm_value, 50) as subject_preview, total_trials, 
       ROUND(total_successes::numeric / NULLIF(total_trials, 0) * 100, 1) as success_rate_pct
FROM sa_bandit_arms 
WHERE arm_type = 'subject_line' AND active = true
ORDER BY total_trials DESC;

COMMIT;
