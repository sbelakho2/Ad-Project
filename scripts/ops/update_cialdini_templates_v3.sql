-- ============================================================================
-- Cialdini-Aligned Email Templates v3 (Grammar-Safe Refactoring)
-- Date: January 2026
-- Purpose: Sentence-level spintax instead of word-level to prevent grammar errors
-- Based on: Cialdini's Influence + Pre-Suasion + Production Feedback
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
--   2. AUTHORITY: Specific benchmarks ({{LIFT_PERCENT}}% or fallback 15-30%)
--   3. LIKING: Use icebreaker to show genuine interest
--   4. SOCIAL PROOF: Reference what similar apps achieve
--   5. PRE-SUASION: Subject primes "free" and "value"
-- 
-- FIX v3: Sentence-level variations to prevent grammar errors like "I ran a analysis"
-- ============================================================================

INSERT INTO sa_spintax_templates (
  name, 
  description, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables
) VALUES (
  'Initial Outreach v3 - Cialdini Reciprocity (Grammar-Safe)',
  'Cialdini cold outreach with sentence-level spintax. Reciprocity (free audit), Authority (benchmarks), Liking (icebreaker).',
  'initial_outreach',
  '{Free ad revenue audit for {{APP_NAME}}|{{APP_NAME}} — quick revenue insight|Complimentary eCPM analysis: {{APP_NAME}}}',
  E'{Hi {{FIRST_NAME}},|Hey {{FIRST_NAME}},|{{FIRST_NAME}},}

{{ICEBREAKER}}

{I put together a complimentary ad revenue analysis for {{APP_NAME}} — no strings attached.|I ran a quick analysis on {{APP_NAME}}''s ad monetization and wanted to share it with you.|I recently compiled some ad revenue insights for {{APP_NAME}} that I thought might be useful.}

{Here''s the quick takeaway: {{APP_GENRE}} apps using proper mediation typically see 15-30% higher eCPMs compared to single-network setups.|The key insight: apps in your category that optimize their waterfall often hit 20-35% better fill rates.|What stood out: {{APP_GENRE}} publishers with multi-network demand sources are averaging significantly higher revenue per impression.}

{I''d be happy to send over the full breakdown — it includes specific opportunities for {{APP_NAME}}.|Want me to walk you through the detailed analysis? It covers exact optimization points for your category.|If useful, I can share the complete report with actionable suggestions tailored to {{APP_GENRE}} apps.}

{Either way, impressive work with {{APP_NAME}}.|Regardless, congrats on what you''ve built.|No pressure either way — great job on {{APP_NAME}}.}

Cheers,
Aaron
ApexMediation',
  '["first_name", "company_name", "app_name", "app_genre", "icebreaker", "lift_percent"]'
);

-- ============================================================================
-- TEMPLATE 2: Follow-up #1 (Social Proof + Commitment)
-- ============================================================================
-- Key Principles Applied:
--   1. SOCIAL PROOF: Specific success story from similar publisher
--   2. COMMITMENT: Reference first outreach (they're in a conversation)
--   3. AUTHORITY: Specific numbers (28% increase, $15K milestone)
--   4. RECIPROCITY: Still offering free resource
-- 
-- FIX v3: Complete sentence alternatives to maintain grammatical integrity
-- ============================================================================

INSERT INTO sa_spintax_templates (
  name, 
  description, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables
) VALUES (
  'Follow-up v3 - Social Proof Story (Grammar-Safe)',
  'Cialdini follow-up with sentence-level spintax. Social Proof (success story), Commitment (reference first touch).',
  'followup',
  '{Re: {{APP_NAME}} revenue analysis|Quick update on {{APP_NAME}}|Following up: {{APP_NAME}} insights}',
  E'{Hi {{FIRST_NAME}},|Hey {{FIRST_NAME}},|{{FIRST_NAME}},}

{Wanted to share a relevant story: a {{APP_GENRE}} publisher we work with just hit a 28% eCPM increase — took about 6 weeks.|Thought you''d find this interesting: another app similar to {{APP_NAME}} recently crossed the $15K/month milestone after optimizing their waterfall.|Quick follow-up with some timely news: a {{APP_GENRE}} developer in our network saw 40% better fill rates within their first quarter.}

{The key change? They switched from single-network to multi-source demand.|What made the difference? Adding 3 demand sources to their waterfall setup.|Their approach was simple: they optimized ad placements using audit data like what I put together for you.}

{I still have the {{APP_NAME}} analysis ready — happy to send it over.|Your {{APP_NAME}} report is still on file if you want me to share it.|The analysis I mentioned is still available — would you find it useful?}

{No commitment needed, just sharing what I found.|Genuinely think there''s opportunity here, but no pressure.|Let me know if timing works better now.}

{Best,|Cheers,}
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
-- 
-- FIX v3: Preserves emotional resonance while ensuring grammatical correctness
-- ============================================================================

INSERT INTO sa_spintax_templates (
  name, 
  description, 
  template_type, 
  subject_spintax, 
  body_spintax, 
  available_variables
) VALUES (
  'Final Follow-up v3 - Scarcity + Liking (Grammar-Safe)',
  'Cialdini final touch with sentence-level spintax. Scarcity (limited capacity), Liking (genuine well-wishes).',
  'followup_final',
  '{{APP_NAME}} — {last note from me|closing the loop|final check-in}',
  E'{{FIRST_NAME}},

{Last quick note: our audit queue is getting full for the next few weeks, but I kept your file open because {{APP_NAME}} genuinely stood out.|Final message from me on this — things are busy, though I wanted to circle back one more time since {{APP_NAME}} caught my attention.|Keeping this short: we''re at capacity this month, but I circled back because {{APP_NAME}} genuinely impressed me.}

{If timing isn''t right now, no worries at all — completely get it.|Totally understand if you''re focused elsewhere — all good.|If this isn''t a priority right now, no pressure whatsoever.}

{But if you''re curious, I can squeeze in a quick chat before things get too hectic.|However, if there''s any interest, let me know and I''ll prioritize your audit this week.|Though if you want to explore this while I have bandwidth, I''m happy to make time.}

{Either way, wishing {{APP_NAME}} continued success — keep building.|Regardless, best of luck with {{APP_NAME}} — excited to see where it goes.|In any case, rooting for you — keep shipping.}

{Take care,|Best,|Cheers,}
Aaron',
  '["first_name", "company_name", "app_name", "app_genre", "icebreaker"]'
);

-- Verify the new templates
SELECT id, name, template_type, active, LEFT(subject_spintax, 60) as subject_preview
FROM sa_spintax_templates 
WHERE active = true
ORDER BY created_at DESC;

COMMIT;
