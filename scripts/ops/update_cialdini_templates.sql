-- ============================================================================
-- Cialdini-Aligned Email Templates Update
-- Date: January 2026
-- Purpose: Replace amateur templates with psychologically-optimized versions
-- Based on: Cialdini's Influence (6 Principles) + Pre-Suasion + Toyota Sales
-- ============================================================================

BEGIN;

-- First, deactivate existing templates (preserve history)
UPDATE sa_spintax_templates 
SET active = false, updated_at = CURRENT_TIMESTAMP 
WHERE active = true;

-- ============================================================================
-- TEMPLATE 1: Initial Outreach (Cialdini Reciprocity + Authority + Liking)
-- ============================================================================
-- Key Principles Applied:
--   1. RECIPROCITY: Lead with free value (audit), not ask
--   2. AUTHORITY: Specific benchmarks (15-30% eCPM lift)
--   3. LIKING: Use icebreaker to show genuine interest
--   4. SOCIAL PROOF: Reference what similar apps achieve
--   5. PRE-SUASION: Subject primes "free" and "value"
-- ============================================================================

INSERT INTO sa_spintax_templates (
  name, 
  description, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables
) VALUES (
  'Initial Outreach v2 - Cialdini Reciprocity',
  'Cialdini-aligned cold outreach: Reciprocity (free audit offer), Authority (specific benchmarks), Liking (icebreaker personalization), Social Proof (category stats)',
  'initial_outreach',
  '{Free ad audit for {{APP_NAME}}|{{APP_NAME}} - quick revenue insight|Complimentary eCPM analysis: {{APP_NAME}}}',
  E'{Hi|Hey} {{FIRST_NAME}},

{{ICEBREAKER}}

{I ran|I just ran|I put together} a {quick|complimentary|free} ad revenue analysis for {{APP_NAME}} - {no strings attached|thought it might be useful|wanted to share it with you}.

{Here''s what I found|Quick highlights|The takeaway}: {{APP_GENRE}} apps {that optimize their waterfall|using mediation properly|with multi-network setups} {typically see|are averaging|often hit} 15-30% {higher eCPMs|better fill rates|revenue lift} {compared to single-network setups|vs direct integrations|over baseline}.

{I''d be happy to|If useful, I can|Want me to} {send over|share|walk you through} the {full breakdown|detailed analysis|complete report}? {It includes|Covers|Shows} {specific opportunities|actionable suggestions|exact optimization points} for {your category|{{APP_GENRE}} apps|apps like yours}.

{Either way|Regardless|No pressure} - {congrats on|impressive work with|great job on} {{APP_NAME}}.

{Cheers|Best|Thanks},
Aaron
ApexMediation',
  '["first_name", "company_name", "app_name", "app_genre", "icebreaker"]'
);

-- ============================================================================
-- TEMPLATE 2: Follow-up #1 (Social Proof + Commitment)
-- ============================================================================
-- Key Principles Applied:
--   1. SOCIAL PROOF: Specific success story from similar publisher
--   2. COMMITMENT: Reference first outreach (they're in a conversation)
--   3. AUTHORITY: Specific numbers (28% increase, $15K milestone)
--   4. RECIPROCITY: Still offering free resource
-- ============================================================================

INSERT INTO sa_spintax_templates (
  name, 
  description, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables
) VALUES (
  'Follow-up v2 - Social Proof Story',
  'Cialdini follow-up: Social Proof (similar publisher success story), Commitment (reference first touch), Authority (specific numbers)',
  'followup',
  '{Re: {{APP_NAME}} revenue analysis|Quick update on {{APP_NAME}}|Following up: {{APP_NAME}} insights}',
  E'{Hi|Hey} {{FIRST_NAME}},

{Wanted to share|Thought you''d find this interesting|Quick follow-up with} a {relevant|timely|recent} story:

{A|Another} {{APP_GENRE}} {publisher|app|developer} {we work with|in our network|similar to {{APP_NAME}}} {just hit|recently reached|crossed} {a 28% eCPM increase|$15K/month milestone|40% better fill rates} - {took about 6 weeks|within their first quarter|after optimizing their waterfall}.

{The key change?|What made the difference?|Their approach?} {They|Simply} {switched from single-network to multi-source demand|added 3 demand sources to their waterfall|optimized ad placements using audit data}.

{I still have|Your|The} {{APP_NAME}} analysis {ready|on file|available} - {happy to send it over|want me to share it|would you find it useful}? {No commitment needed|Just sharing what I found|Genuinely think there''s opportunity here}.

{Let me know|Thoughts?|Any interest?}

{Best|Cheers},
Aaron',
  '["first_name", "company_name", "app_name", "app_genre", "icebreaker"]'
);

-- ============================================================================
-- TEMPLATE 3: Final Follow-up (Scarcity + Liking + Unity)
-- ============================================================================
-- Key Principles Applied:
--   1. SCARCITY: Limited capacity, closing the loop
--   2. LIKING: Genuine compliment, partnership framing
--   3. UNITY: "We" language, shared goals
--   4. COMMITMENT: Low-friction next step
-- ============================================================================

INSERT INTO sa_spintax_templates (
  name, 
  description, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables
) VALUES (
  'Final Follow-up v2 - Scarcity + Liking',
  'Cialdini final touch: Scarcity (limited capacity), Liking (genuine well-wishes), Commitment (low-friction next step)',
  'followup_final',
  '{{APP_NAME}} - {last note from me|closing the loop|final check-in}',
  E'{{FIRST_NAME}},

{Last quick note|Final message from me on this|Keeping this short}:

{We''re at capacity|Our audit queue is full|Things are busy} for {the next few weeks|February|this month} - {but|though} I {wanted to reach out|kept your file open|circled back} {one more time|before closing this out} {because|since} {{APP_NAME}} {genuinely impressed me|caught my attention|stood out}.

{If timing isn''t right|If this isn''t a priority now|Totally understand if you''re focused elsewhere} - {no worries at all|completely get it|all good}. {But if|However, if|Though if} {you''re curious|there''s any interest|you want to explore this}, {I''m happy to|I can|let me} {squeeze in a quick chat|make time|prioritize your audit} {before things get too hectic|this week|while I have bandwidth}.

{Either way|Regardless|In any case}, {wishing|best of luck to|rooting for} {{APP_NAME}} - {keep building|keep shipping|excited to see where it goes}.

{Best|Cheers|Take care},
Aaron',
  '["first_name", "company_name", "app_name", "app_genre", "icebreaker"]'
);

-- Verify the new templates
SELECT id, name, template_type, active, LEFT(subject_spintax, 60) as subject_preview
FROM sa_spintax_templates 
ORDER BY active DESC, created_at DESC;

COMMIT;
