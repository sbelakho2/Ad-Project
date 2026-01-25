-- Migration: 049_autonomous_sales_v2.sql
-- Zero-Cost Autonomous Sales System V2
-- Implements: Hunter (Sourcing), Analyst (Heuristics), Writer (Spintax), 
--             Optimizer (Thompson Sampling), Clerk (Inbox Classification)

-- ============================================================================
-- 1. SOURCING ENGINE (The Hunter)
-- Tracks where leads were discovered and scraping jobs
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_sources (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Source identification
  source_type VARCHAR(50) NOT NULL, -- 'google_play', 'app_store', 'app_ads_txt', 'website_crawl', 'manual'
  source_url TEXT,
  source_name VARCHAR(255),
  
  -- Scrape configuration
  search_query TEXT, -- For app store searches
  category VARCHAR(100), -- App store category
  region VARCHAR(10) DEFAULT 'us', -- Region code
  min_downloads VARCHAR(20), -- e.g., '10K'
  max_downloads VARCHAR(20), -- e.g., '10M'
  
  -- Status
  status VARCHAR(20) DEFAULT 'active', -- 'active', 'paused', 'exhausted', 'failed'
  last_scraped_at TIMESTAMP,
  next_scrape_at TIMESTAMP,
  scrape_interval_hours INTEGER DEFAULT 24,
  
  -- Stats
  total_leads_found INTEGER DEFAULT 0,
  total_qualified INTEGER DEFAULT 0,
  total_contacted INTEGER DEFAULT 0,
  success_rate DECIMAL(5,2) DEFAULT 0,
  
  -- Error tracking
  consecutive_failures INTEGER DEFAULT 0,
  last_error TEXT,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_sources_status ON sa_sources(status);
CREATE INDEX IF NOT EXISTS idx_sa_sources_next_scrape ON sa_sources(next_scrape_at) WHERE status = 'active';

-- Scrape job history
CREATE TABLE IF NOT EXISTS sa_scrape_jobs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source_id UUID REFERENCES sa_sources(id) ON DELETE CASCADE,
  
  -- Job details
  started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  completed_at TIMESTAMP,
  status VARCHAR(20) DEFAULT 'running', -- 'running', 'completed', 'failed', 'rate_limited'
  
  -- Results
  pages_scraped INTEGER DEFAULT 0,
  apps_found INTEGER DEFAULT 0,
  contacts_extracted INTEGER DEFAULT 0,
  emails_found INTEGER DEFAULT 0,
  
  -- Errors
  error_message TEXT,
  error_stack TEXT,
  
  -- Performance
  duration_ms INTEGER,
  memory_peak_mb INTEGER
);

CREATE INDEX IF NOT EXISTS idx_sa_scrape_jobs_source ON sa_scrape_jobs(source_id);
CREATE INDEX IF NOT EXISTS idx_sa_scrape_jobs_status ON sa_scrape_jobs(status);

-- ============================================================================
-- 2. HEURISTIC SCORING (The Analyst)
-- Stores computed scores and signals for each contact
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_heuristic_scores (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID NOT NULL,
  
  -- Individual signal scores (0-100)
  app_ads_txt_score INTEGER DEFAULT 0, -- Has app-ads.txt with multiple networks
  sdk_diversity_score INTEGER DEFAULT 0, -- Multiple ad SDKs detected
  update_velocity_score INTEGER DEFAULT 0, -- Recent updates = active team
  review_velocity_score INTEGER DEFAULT 0, -- Growing reviews = growing app
  tech_sophistication_score INTEGER DEFAULT 0, -- Firebase, Segment, etc.
  company_size_score INTEGER DEFAULT 0, -- Inferred from signals
  
  -- Raw signals (JSONB for flexibility)
  signals JSONB DEFAULT '{}',
  -- Example: {"app_ads_txt": ["google.com", "applovin.com"], "sdks_detected": ["firebase", "adjust"]}
  
  -- Composite scores
  overall_score INTEGER GENERATED ALWAYS AS (
    GREATEST(0, LEAST(100, (
      app_ads_txt_score * 0.25 +
      sdk_diversity_score * 0.20 +
      update_velocity_score * 0.15 +
      review_velocity_score * 0.15 +
      tech_sophistication_score * 0.15 +
      company_size_score * 0.10
    )::INTEGER))
  ) STORED,
  
  -- Classification
  tier VARCHAR(20) GENERATED ALWAYS AS (
    CASE 
      WHEN (app_ads_txt_score * 0.25 + sdk_diversity_score * 0.20 + update_velocity_score * 0.15 + review_velocity_score * 0.15 + tech_sophistication_score * 0.15 + company_size_score * 0.10) >= 70 THEN 'whale'
      WHEN (app_ads_txt_score * 0.25 + sdk_diversity_score * 0.20 + update_velocity_score * 0.15 + review_velocity_score * 0.15 + tech_sophistication_score * 0.15 + company_size_score * 0.10) >= 40 THEN 'qualified'
      ELSE 'cold'
    END
  ) STORED,
  
  -- Metadata
  last_analyzed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  analysis_version INTEGER DEFAULT 1,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  CONSTRAINT fk_contact FOREIGN KEY (contact_id) REFERENCES sa_contacts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_sa_heuristic_overall ON sa_heuristic_scores(overall_score DESC);
CREATE INDEX IF NOT EXISTS idx_sa_heuristic_tier ON sa_heuristic_scores(tier);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sa_heuristic_contact ON sa_heuristic_scores(contact_id);

-- ============================================================================
-- 3. SPINTAX TEMPLATES (The Writer)
-- Stores templates with spinning syntax for infinite variations
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_spintax_templates (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Template identification
  name VARCHAR(255) NOT NULL,
  description TEXT,
  template_type VARCHAR(50) DEFAULT 'email', -- 'email', 'subject', 'followup'
  
  -- The spintax content (uses {option1|option2|option3} syntax)
  subject_spintax TEXT NOT NULL,
  body_spintax TEXT NOT NULL,
  
  -- Available variables
  available_variables JSONB DEFAULT '["first_name", "company_name", "app_name", "app_genre", "icebreaker"]',
  
  -- Template statistics
  times_used INTEGER DEFAULT 0,
  total_opens INTEGER DEFAULT 0,
  total_clicks INTEGER DEFAULT 0,
  total_replies INTEGER DEFAULT 0,
  open_rate DECIMAL(5,2) GENERATED ALWAYS AS (
    CASE WHEN times_used > 0 THEN (total_opens::DECIMAL / times_used) * 100 ELSE 0 END
  ) STORED,
  reply_rate DECIMAL(5,2) GENERATED ALWAYS AS (
    CASE WHEN times_used > 0 THEN (total_replies::DECIMAL / times_used) * 100 ELSE 0 END
  ) STORED,
  
  -- Status
  active BOOLEAN DEFAULT true,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_spintax_active ON sa_spintax_templates(active) WHERE active = true;
CREATE INDEX IF NOT EXISTS idx_sa_spintax_type ON sa_spintax_templates(template_type);

-- Resolved spintax history (for anti-spam analysis)
CREATE TABLE IF NOT EXISTS sa_spintax_history (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  template_id UUID REFERENCES sa_spintax_templates(id) ON DELETE CASCADE,
  email_id UUID,
  
  -- The resolved content (what was actually sent)
  resolved_subject TEXT NOT NULL,
  resolved_body TEXT NOT NULL,
  
  -- Hash for similarity detection
  content_hash VARCHAR(64) NOT NULL, -- SHA-256 of normalized content
  
  sent_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_spintax_history_hash ON sa_spintax_history(content_hash);
CREATE INDEX IF NOT EXISTS idx_sa_spintax_history_sent ON sa_spintax_history(sent_at DESC);

-- ============================================================================
-- 4. THOMPSON SAMPLING (The Optimizer)
-- Multi-armed bandit for subject line optimization
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_bandit_arms (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Arm identification
  arm_type VARCHAR(50) NOT NULL, -- 'subject_line', 'template', 'send_time', 'cta'
  arm_name VARCHAR(255) NOT NULL,
  arm_value TEXT NOT NULL, -- The actual content/value
  
  -- Beta distribution parameters (Bayesian priors)
  alpha INTEGER DEFAULT 1, -- Successes + 1 (prior)
  beta INTEGER DEFAULT 1, -- Failures + 1 (prior)
  
  -- Raw counts
  total_trials INTEGER DEFAULT 0,
  total_successes INTEGER DEFAULT 0, -- Opens for subjects, replies for templates
  
  -- Computed metrics
  empirical_rate DECIMAL(5,4) GENERATED ALWAYS AS (
    CASE WHEN total_trials > 0 THEN total_successes::DECIMAL / total_trials ELSE 0 END
  ) STORED,
  
  -- Thompson sampling score (recomputed each time)
  last_sampled_score DECIMAL(10,8),
  last_sampled_at TIMESTAMP,
  
  -- Status
  active BOOLEAN DEFAULT true,
  min_trials_before_retirement INTEGER DEFAULT 100,
  retired_at TIMESTAMP,
  retirement_reason VARCHAR(255),
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_bandit_type ON sa_bandit_arms(arm_type) WHERE active = true;
CREATE INDEX IF NOT EXISTS idx_sa_bandit_score ON sa_bandit_arms(last_sampled_score DESC NULLS LAST);

-- Bandit trial history
CREATE TABLE IF NOT EXISTS sa_bandit_trials (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  arm_id UUID REFERENCES sa_bandit_arms(id) ON DELETE CASCADE,
  email_id UUID,
  contact_id UUID,
  
  -- Trial outcome
  selected_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  outcome_recorded_at TIMESTAMP,
  success BOOLEAN, -- null until recorded
  
  -- Context at selection time
  sampled_score DECIMAL(10,8), -- What score caused this selection
  competing_arms_count INTEGER,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_bandit_trials_arm ON sa_bandit_trials(arm_id);
CREATE INDEX IF NOT EXISTS idx_sa_bandit_trials_pending ON sa_bandit_trials(arm_id) WHERE success IS NULL;

-- ============================================================================
-- 5. INBOX CLASSIFICATION (The Clerk)
-- Automatic categorization of incoming replies
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_inbox_rules (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Rule identification
  name VARCHAR(255) NOT NULL,
  priority INTEGER DEFAULT 100, -- Lower = higher priority
  
  -- Matching criteria
  match_type VARCHAR(50) NOT NULL, -- 'header', 'subject', 'body', 'sender'
  match_field VARCHAR(100), -- e.g., 'Auto-Submitted', 'X-Auto-Response-Suppress'
  match_pattern TEXT NOT NULL, -- Regex pattern
  match_flags VARCHAR(10) DEFAULT 'i', -- Regex flags (i = case insensitive)
  
  -- Classification result
  classification VARCHAR(50) NOT NULL, -- 'ooo', 'bounce', 'unsubscribe', 'interested', 'not_interested', 'meeting_request'
  confidence INTEGER DEFAULT 100, -- How confident is this rule (0-100)
  
  -- Action
  auto_action VARCHAR(50), -- 'archive', 'pause_sequence', 'opt_out', 'alert', 'none'
  alert_slack_channel VARCHAR(100),
  
  -- Status
  active BOOLEAN DEFAULT true,
  times_matched INTEGER DEFAULT 0,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_inbox_rules_priority ON sa_inbox_rules(priority) WHERE active = true;

-- Classified inbox messages
CREATE TABLE IF NOT EXISTS sa_inbox_messages (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID,
  email_id UUID, -- The original outbound email this replies to
  
  -- Message content
  from_email VARCHAR(255) NOT NULL,
  subject TEXT,
  body_text TEXT,
  body_html TEXT,
  received_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  -- Headers (for OOO detection)
  headers JSONB DEFAULT '{}',
  
  -- Classification
  classification VARCHAR(50), -- 'ooo', 'bounce', 'unsubscribe', 'interested', 'not_interested', 'meeting_request', 'unknown'
  classification_confidence INTEGER,
  matched_rule_id UUID REFERENCES sa_inbox_rules(id),
  
  -- Sentiment analysis (local Bayes)
  sentiment_score DECIMAL(5,2), -- -1 to +1
  sentiment_label VARCHAR(20), -- 'positive', 'negative', 'neutral'
  
  -- Processing
  processed BOOLEAN DEFAULT false,
  processed_at TIMESTAMP,
  auto_action_taken VARCHAR(50),
  requires_human_review BOOLEAN DEFAULT false,
  human_reviewed_at TIMESTAMP,
  human_reviewer VARCHAR(255),
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_inbox_unprocessed ON sa_inbox_messages(processed, received_at) WHERE NOT processed;
CREATE INDEX IF NOT EXISTS idx_sa_inbox_classification ON sa_inbox_messages(classification);
CREATE INDEX IF NOT EXISTS idx_sa_inbox_review ON sa_inbox_messages(requires_human_review) WHERE requires_human_review;

-- Bayes classifier training data
CREATE TABLE IF NOT EXISTS sa_bayes_training (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Training sample
  text_content TEXT NOT NULL,
  label VARCHAR(50) NOT NULL, -- 'interested', 'not_interested', 'ooo', 'bounce', etc.
  
  -- Metadata
  source VARCHAR(100), -- 'manual', 'auto_verified', 'inbox_rule'
  verified BOOLEAN DEFAULT false,
  verified_by VARCHAR(255),
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_bayes_label ON sa_bayes_training(label);

-- ============================================================================
-- 6. AUTONOMOUS JOB QUEUE
-- Background jobs for the autonomous system
-- ============================================================================

CREATE TABLE IF NOT EXISTS sa_autonomous_jobs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Job identification
  job_type VARCHAR(50) NOT NULL, -- 'scrape', 'enrich', 'analyze', 'send', 'classify', 'optimize'
  job_name VARCHAR(255),
  
  -- Scheduling
  scheduled_at TIMESTAMP NOT NULL,
  started_at TIMESTAMP,
  completed_at TIMESTAMP,
  
  -- Status
  status VARCHAR(20) DEFAULT 'pending', -- 'pending', 'running', 'completed', 'failed', 'cancelled'
  attempts INTEGER DEFAULT 0,
  max_attempts INTEGER DEFAULT 3,
  
  -- Job data
  input_data JSONB DEFAULT '{}',
  output_data JSONB DEFAULT '{}',
  
  -- Error handling
  error_message TEXT,
  error_stack TEXT,
  
  -- Performance
  duration_ms INTEGER,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sa_jobs_pending ON sa_autonomous_jobs(scheduled_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_sa_jobs_type ON sa_autonomous_jobs(job_type, status);

-- ============================================================================
-- 7. SEED DATA: Default inbox rules
-- ============================================================================

INSERT INTO sa_inbox_rules (name, priority, match_type, match_field, match_pattern, classification, auto_action, active) VALUES
  -- Out of Office detection (high priority)
  ('OOO - Auto-Submitted Header', 10, 'header', 'Auto-Submitted', 'auto-(replied|generated)', 'ooo', 'pause_sequence', true),
  ('OOO - Precedence Header', 11, 'header', 'Precedence', 'bulk|auto_reply', 'ooo', 'pause_sequence', true),
  ('OOO - Subject Keywords', 12, 'subject', null, 'out\s+of\s+(the\s+)?office|auto(matic)?\s*reply|vacation|traveling|abroad|limited\s+access|away\s+from|on\s+leave', 'ooo', 'pause_sequence', true),
  
  -- Bounce detection
  ('Bounce - Failed Delivery Subject', 20, 'subject', null, 'undeliverable|delivery\s+(failed|status|notification)|mail\s+delivery|returned\s+mail|failure\s+notice', 'bounce', 'opt_out', true),
  ('Bounce - SMTP Error Body', 21, 'body', null, '550\s+5\.1\.1|recipient\s+(rejected|not\s+found)|mailbox\s+(not\s+found|unavailable)|user\s+unknown|address\s+rejected', 'bounce', 'opt_out', true),
  
  -- Unsubscribe detection
  ('Unsubscribe - Keywords', 30, 'body', null, '\b(unsubscribe|remove\s+me|stop\s+emailing|opt\s*out|take\s+me\s+off)\b', 'unsubscribe', 'opt_out', true),
  
  -- Interest detection (positive signals)
  ('Interested - Meeting Request', 40, 'body', null, '\b(let''?s?\s+(talk|chat|connect|meet|schedule)|book\s+a\s+call|set\s+up\s+a\s+(call|meeting)|available\s+(to\s+chat|this\s+week)|sounds?\s+interesting|tell\s+me\s+more)\b', 'interested', 'alert', true),
  ('Interested - Pricing/Demo Request', 41, 'body', null, '\b(pricing|demo|trial|how\s+much|what\s+does\s+it\s+cost|send\s+me\s+(info|details|more))\b', 'interested', 'alert', true),
  
  -- Not interested detection (negative signals)
  ('Not Interested - Explicit', 50, 'body', null, '\b(not\s+interested|no\s+thanks?|not\s+for\s+us|not\s+a\s+fit|not\s+looking|already\s+have|happy\s+with\s+(our\s+)?current|pass\s+on\s+this)\b', 'not_interested', 'archive', true)
  
ON CONFLICT DO NOTHING;

-- ============================================================================
-- 8. SEED DATA: Default spintax templates
-- ============================================================================

INSERT INTO sa_spintax_templates (name, description, template_type, subject_spintax, body_spintax, available_variables) VALUES
(
  'Initial Outreach - Ad Mediation',
  'First touch template for app developers using ad monetization',
  'email',
  '{Quick question about|Question re:|Regarding} {{app_name}} {monetization|ad revenue|ad strategy}',
  E'{Hi|Hello|Hey} {{first_name}},

{I noticed|I came across|I saw} {{app_name}} on the {App Store|Play Store|app stores} and {was impressed by|loved|really liked} your {approach to|work on} {{app_genre}}.

{I''m reaching out because|The reason for my email is|Writing to you as} we {help|specialize in helping|work with} {apps like yours|mobile publishers|developers} {optimize|increase|boost} their {ad revenue|monetization|yield} {without adding SDK bloat|with a single integration|effortlessly}.

{Our platform|ApexMediation} typically {increases|improves|boosts} {eCPM|revenue|fill rates} by {15-40%|20%+|significantly} for {apps in your space|similar publishers|{{app_genre}} apps}.

{Would you be open to|Are you free for|Do you have 15 minutes for} a {quick chat|brief call|short conversation} to see if {we''d be a fit|this makes sense|there''s alignment}?

{Best|Cheers|Thanks},
{Aaron|The ApexMediation Team}',
  '["first_name", "app_name", "app_genre", "company_name"]'
),
(
  'Follow-up #1 - Value Add',
  'First follow-up with additional value',
  'followup',
  '{Re: |Following up: |Quick follow-up on }{{app_name}}',
  E'{Hi|Hey} {{first_name}},

{Just wanted to follow up|Circling back|Bumping this up} on my {previous email|last message|note from last week}.

{I did some research and|After looking into it|I noticed that} {{app_name}} {could benefit from|might be leaving revenue on the table with|has room to optimize} {your current ad setup|mediation|monetization strategy}.

{Happy to share|I''d love to show you|Can send over} some {specific insights|data|benchmarks} for {{{app_genre}} apps|your vertical|similar publishers} if {helpful|that''d be useful|you''re interested}.

{Let me know|Thoughts?|Any interest?}

{Best|Cheers},
{Aaron|ApexMediation}',
  '["first_name", "app_name", "app_genre"]'
),
(
  'Follow-up #2 - Final',
  'Final follow-up before closing',
  'followup',
  '{Last attempt|Final check-in|Closing the loop}: {{app_name}}',
  E'{{first_name}},

{I''ll keep this short|Quick one|Last message from me on this} - {I know you''re busy|don''t want to spam you|I get it if timing isn''t right}.

{If ad monetization|If optimizing yield|If mediation} {isn''t a priority right now|is on the back burner|isn''t top of mind}, {totally understand|no worries at all|completely get it}.

{But if it is|If you''re curious|However, if there''s interest}, {I''m here|happy to chat|would love to connect} whenever {makes sense|you have a moment|timing works better}.

{Either way, wishing|Regardless, best of luck with|In any case, cheering on} {{app_name}}!

{Best|Cheers|Take care},
{Aaron}',
  '["first_name", "app_name"]'
)
ON CONFLICT DO NOTHING;

-- ============================================================================
-- 9. Add source tracking to contacts
-- ============================================================================

ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS source_id UUID REFERENCES sa_sources(id);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_store VARCHAR(50);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_id VARCHAR(255);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_genre VARCHAR(100);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS icebreaker TEXT;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS website_meta_description TEXT;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_ads_txt_content TEXT;

CREATE INDEX IF NOT EXISTS idx_sa_contacts_source ON sa_contacts(source_id);
