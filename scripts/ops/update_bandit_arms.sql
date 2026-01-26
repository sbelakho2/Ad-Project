-- ============================================================================
-- Pre-Suasion Aware Subject Line Bandit Arms Update
-- Date: January 2026
-- Purpose: Replace weak subject lines with psychologically-optimized versions
-- Based on: Cialdini's Pre-Suasion (attention priming) + Influence principles
-- ============================================================================

BEGIN;

-- Deactivate existing bandit arms (preserve trial data)
UPDATE sa_bandit_arms 
SET active = false, retired_at = CURRENT_TIMESTAMP, retirement_reason = 'Replaced with Cialdini pre-suasion optimized arms'
WHERE active = true AND arm_type = 'subject_line';

-- ============================================================================
-- NEW PRE-SUASION AWARE SUBJECT LINE ARMS
-- ============================================================================
-- Key Principles:
--   1. RECIPROCITY PRIMING: "Free", "Complimentary", "For you"
--   2. CURIOSITY GAP: Incomplete information that demands resolution
--   3. SOCIAL PROOF: Numbers, "other publishers", "similar apps"
--   4. AUTHORITY: Specific data, professional framing
--   5. PERSONALIZATION: App name, genre specificity
--   6. SCARCITY: Limited, exclusive, time-sensitive
-- ============================================================================

-- ARM 1: Reciprocity Prime (Free value)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'reciprocity_free_audit', 'Free ad audit for {{APP_NAME}}', 1, 1, 50);

-- ARM 2: Reciprocity Prime (Complimentary)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'reciprocity_complimentary', 'Complimentary eCPM analysis: {{APP_NAME}}', 1, 1, 50);

-- ARM 3: Curiosity Gap (Revenue insight)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'curiosity_revenue', '{{APP_NAME}} - quick revenue insight', 1, 1, 50);

-- ARM 4: Curiosity Gap (Found something)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'curiosity_found', 'Found something interesting about {{APP_NAME}}', 1, 1, 50);

-- ARM 5: Social Proof (Similar apps)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'social_proof_similar', 'How {{APP_GENRE}} apps like yours increase eCPM', 1, 1, 50);

-- ARM 6: Social Proof (Specific number)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'social_proof_number', '37 {{APP_GENRE}} publishers saw this last month', 1, 1, 50);

-- ARM 7: Authority (Benchmark data)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'authority_benchmark', '{{APP_NAME}} vs {{APP_GENRE}} benchmarks', 1, 1, 50);

-- ARM 8: Scarcity (Limited spots)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'scarcity_limited', 'Limited audit slots for {{APP_GENRE}} apps this week', 1, 1, 50);

-- ARM 9: Personalized Question (Engagement)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'question_personalized', 'Quick question about {{APP_NAME}} ad setup', 1, 1, 50);

-- ARM 10: Direct Value Prop (Specific benefit)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'value_prop_lift', '15-30% eCPM lift opportunity for {{APP_NAME}}', 1, 1, 50);

-- ARM 11: Unity/Partnership (We language)
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'unity_partnership', '{{APP_NAME}} + ApexMediation - worth exploring?', 1, 1, 50);

-- ARM 12: Pre-suasion Pattern Interrupt
INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta, min_trials_before_retirement)
VALUES ('subject_line', 'pattern_interrupt', 'Not another ad network pitch - {{FIRST_NAME}}', 1, 1, 50);

-- Verify the new arms
SELECT id, arm_name, arm_value, active, total_trials
FROM sa_bandit_arms 
WHERE arm_type = 'subject_line'
ORDER BY active DESC, created_at DESC;

COMMIT;
