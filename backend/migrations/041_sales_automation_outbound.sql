-- Migration: Sales Automation (Zero-Cost Outbound)
-- Creates tables for automated prospect discovery, enrichment, and outreach

-- ============================================================================
-- PROSPECTS & COMPANIES
-- ============================================================================

-- Discovered prospects (apps/companies)
CREATE TABLE IF NOT EXISTS sales_prospects (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- App identification
  app_store VARCHAR(20) NOT NULL, -- 'google_play', 'app_store', 'amazon', 'web'
  app_id VARCHAR(255) NOT NULL, -- Bundle ID or package name
  app_name VARCHAR(500),
  app_url TEXT,
  
  -- Company identification
  developer_name VARCHAR(500),
  developer_id VARCHAR(255),
  company_domain VARCHAR(255),
  company_name VARCHAR(500),
  
  -- App metrics (public data)
  app_category VARCHAR(100),
  downloads_estimate VARCHAR(50), -- '10K-50K', '1M-5M', etc.
  rating DECIMAL(3,2),
  review_count INTEGER,
  has_iap BOOLEAN DEFAULT false, -- In-app purchases
  has_ads BOOLEAN DEFAULT false, -- Contains ads
  last_update_date DATE,
  
  -- Discovery metadata
  discovery_source VARCHAR(100), -- 'app_store_search', 'competitor_similar', 'github'
  discovery_date TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  -- Processing status
  status VARCHAR(30) DEFAULT 'discovered', -- 'discovered', 'enriching', 'enriched', 'qualified', 'disqualified', 'contacted', 'converted'
  enrichment_score INTEGER DEFAULT 0, -- 0-100
  fit_score INTEGER DEFAULT 0, -- 0-100 ICP fit
  intent_score INTEGER DEFAULT 0, -- 0-100 buying intent
  
  -- Disqualification
  disqualified_reason VARCHAR(255),
  disqualified_at TIMESTAMP,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(app_store, app_id)
);

CREATE INDEX IF NOT EXISTS idx_prospects_status ON sales_prospects(status);
CREATE INDEX IF NOT EXISTS idx_prospects_domain ON sales_prospects(company_domain);
CREATE INDEX IF NOT EXISTS idx_prospects_scores ON sales_prospects(fit_score DESC, intent_score DESC);
CREATE INDEX IF NOT EXISTS idx_prospects_discovery_date ON sales_prospects(discovery_date DESC);

-- Enriched company data
CREATE TABLE IF NOT EXISTS sales_companies (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  domain VARCHAR(255) NOT NULL UNIQUE,
  
  -- Basic info
  name VARCHAR(500),
  description TEXT,
  industry VARCHAR(100),
  
  -- Size indicators
  employee_count_range VARCHAR(50), -- '1-10', '11-50', '51-200', '201-500', '500+'
  estimated_revenue_range VARCHAR(50), -- '<$1M', '$1M-$10M', '$10M-$50M', etc.
  funding_stage VARCHAR(50), -- 'bootstrapped', 'seed', 'series_a', 'series_b+', 'public'
  
  -- Location
  country_code CHAR(2),
  city VARCHAR(100),
  timezone VARCHAR(50),
  
  -- Tech signals
  tech_stack JSONB DEFAULT '[]', -- Detected technologies
  ad_networks JSONB DEFAULT '[]', -- Current ad networks in use
  mediation_platform VARCHAR(100), -- Current mediation (if detected)
  
  -- Social presence
  linkedin_url TEXT,
  twitter_handle VARCHAR(100),
  github_org VARCHAR(100),
  
  -- Enrichment metadata
  enrichment_sources JSONB DEFAULT '[]',
  last_enriched_at TIMESTAMP,
  enrichment_quality_score INTEGER DEFAULT 0,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_companies_country ON sales_companies(country_code);
CREATE INDEX IF NOT EXISTS idx_companies_industry ON sales_companies(industry);

-- ============================================================================
-- CONTACTS
-- ============================================================================

-- Discovered contacts at companies
CREATE TABLE IF NOT EXISTS sales_contacts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id UUID REFERENCES sales_companies(id) ON DELETE CASCADE,
  prospect_id UUID REFERENCES sales_prospects(id) ON DELETE SET NULL,
  
  -- Contact info
  email VARCHAR(255),
  email_verified BOOLEAN DEFAULT false,
  email_verification_status VARCHAR(30), -- 'pending', 'valid', 'invalid', 'catch_all', 'unknown'
  
  first_name VARCHAR(100),
  last_name VARCHAR(100),
  full_name VARCHAR(255),
  
  -- Professional info
  job_title VARCHAR(255),
  department VARCHAR(100), -- 'engineering', 'product', 'marketing', 'executive', 'unknown'
  seniority VARCHAR(50), -- 'individual', 'manager', 'director', 'vp', 'c_level'
  
  -- Contact channels
  linkedin_url TEXT,
  github_username VARCHAR(100),
  twitter_handle VARCHAR(100),
  
  -- Scoring
  contact_score INTEGER DEFAULT 0, -- How good is this contact (decision maker, etc.)
  reachability_score INTEGER DEFAULT 0, -- How reachable (verified email, LinkedIn, etc.)
  
  -- Status
  status VARCHAR(30) DEFAULT 'discovered', -- 'discovered', 'verified', 'contacted', 'responded', 'converted', 'opted_out'
  opted_out_at TIMESTAMP,
  
  -- Discovery
  discovery_source VARCHAR(100),
  discovered_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(email)
);

CREATE INDEX IF NOT EXISTS idx_contacts_company ON sales_contacts(company_id);
CREATE INDEX IF NOT EXISTS idx_contacts_email ON sales_contacts(email);
CREATE INDEX IF NOT EXISTS idx_contacts_status ON sales_contacts(status);
CREATE INDEX IF NOT EXISTS idx_contacts_score ON sales_contacts(contact_score DESC);

-- ============================================================================
-- OUTREACH SEQUENCES
-- ============================================================================

-- Outreach sequences (templates)
CREATE TABLE IF NOT EXISTS outreach_sequences (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name VARCHAR(255) NOT NULL,
  description TEXT,
  
  -- Targeting
  target_segment VARCHAR(100), -- 'gaming', 'utility', 'news', 'all'
  target_company_size VARCHAR(50),
  target_region VARCHAR(50),
  
  -- Sequence settings
  total_steps INTEGER NOT NULL,
  step_delays_days INTEGER[] NOT NULL, -- [0, 3, 7, 14] = immediate, then 3, 7, 14 days
  
  -- A/B testing
  is_ab_test BOOLEAN DEFAULT false,
  ab_variant VARCHAR(1), -- 'A', 'B'
  
  -- Status
  active BOOLEAN DEFAULT true,
  
  -- Performance metrics (updated by analytics)
  total_enrolled INTEGER DEFAULT 0,
  total_opened INTEGER DEFAULT 0,
  total_clicked INTEGER DEFAULT 0,
  total_replied INTEGER DEFAULT 0,
  total_converted INTEGER DEFAULT 0,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Sequence steps (individual emails in a sequence)
CREATE TABLE IF NOT EXISTS outreach_sequence_steps (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  sequence_id UUID NOT NULL REFERENCES outreach_sequences(id) ON DELETE CASCADE,
  step_number INTEGER NOT NULL,
  
  -- Email content
  subject_template TEXT NOT NULL,
  body_template TEXT NOT NULL, -- HTML with {{variables}}
  
  -- Personalization tokens available
  -- {{first_name}}, {{company_name}}, {{app_name}}, {{downloads}}, 
  -- {{current_mediation}}, {{estimated_revenue}}, {{pain_point}}
  
  -- Timing
  delay_days INTEGER NOT NULL DEFAULT 0, -- Days after previous step (or enrollment for step 1)
  send_time_preference VARCHAR(20) DEFAULT 'business_hours', -- 'business_hours', 'morning', 'afternoon'
  
  -- A/B variants for this step
  variant_subject TEXT,
  variant_body TEXT,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(sequence_id, step_number)
);

CREATE INDEX IF NOT EXISTS idx_sequence_steps_sequence ON outreach_sequence_steps(sequence_id);

-- Contact enrollments in sequences
CREATE TABLE IF NOT EXISTS outreach_enrollments (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID NOT NULL REFERENCES sales_contacts(id) ON DELETE CASCADE,
  sequence_id UUID NOT NULL REFERENCES outreach_sequences(id) ON DELETE CASCADE,
  prospect_id UUID REFERENCES sales_prospects(id) ON DELETE SET NULL,
  
  -- Enrollment status
  status VARCHAR(30) DEFAULT 'active', -- 'active', 'completed', 'paused', 'responded', 'converted', 'bounced', 'opted_out'
  current_step INTEGER DEFAULT 0, -- 0 = not started, 1 = first email, etc.
  
  -- Timing
  enrolled_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  next_step_at TIMESTAMP,
  completed_at TIMESTAMP,
  
  -- Engagement
  emails_sent INTEGER DEFAULT 0,
  emails_opened INTEGER DEFAULT 0,
  emails_clicked INTEGER DEFAULT 0,
  replied_at TIMESTAMP,
  
  -- Personalization data (snapshot at enrollment time)
  personalization_data JSONB DEFAULT '{}',
  
  -- A/B test assignment
  ab_variant VARCHAR(1),
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(contact_id, sequence_id)
);

CREATE INDEX IF NOT EXISTS idx_enrollments_contact ON outreach_enrollments(contact_id);
CREATE INDEX IF NOT EXISTS idx_enrollments_sequence ON outreach_enrollments(sequence_id);
CREATE INDEX IF NOT EXISTS idx_enrollments_status ON outreach_enrollments(status);
CREATE INDEX IF NOT EXISTS idx_enrollments_next_step ON outreach_enrollments(next_step_at) WHERE status = 'active';

-- Individual outreach emails sent
CREATE TABLE IF NOT EXISTS outreach_emails (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  enrollment_id UUID NOT NULL REFERENCES outreach_enrollments(id) ON DELETE CASCADE,
  step_id UUID NOT NULL REFERENCES outreach_sequence_steps(id) ON DELETE CASCADE,
  contact_id UUID NOT NULL REFERENCES sales_contacts(id) ON DELETE CASCADE,
  
  -- Email details
  to_email VARCHAR(255) NOT NULL,
  from_email VARCHAR(255) NOT NULL,
  subject TEXT NOT NULL,
  body_html TEXT NOT NULL,
  body_text TEXT,
  
  -- Tracking
  message_id VARCHAR(255), -- From email provider
  tracking_pixel_id UUID DEFAULT gen_random_uuid(),
  
  -- Status
  status VARCHAR(30) DEFAULT 'pending', -- 'pending', 'sent', 'delivered', 'opened', 'clicked', 'replied', 'bounced', 'complained'
  
  -- Timestamps
  scheduled_for TIMESTAMP,
  sent_at TIMESTAMP,
  delivered_at TIMESTAMP,
  opened_at TIMESTAMP,
  opened_count INTEGER DEFAULT 0,
  clicked_at TIMESTAMP,
  clicked_count INTEGER DEFAULT 0,
  replied_at TIMESTAMP,
  bounced_at TIMESTAMP,
  bounce_reason TEXT,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_outreach_emails_enrollment ON outreach_emails(enrollment_id);
CREATE INDEX IF NOT EXISTS idx_outreach_emails_contact ON outreach_emails(contact_id);
CREATE INDEX IF NOT EXISTS idx_outreach_emails_status ON outreach_emails(status);
CREATE INDEX IF NOT EXISTS idx_outreach_emails_scheduled ON outreach_emails(scheduled_for) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_outreach_emails_tracking ON outreach_emails(tracking_pixel_id);

-- ============================================================================
-- SALES ACTIVITIES & PIPELINE
-- ============================================================================

-- Sales pipeline stages
CREATE TABLE IF NOT EXISTS sales_deals (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  prospect_id UUID REFERENCES sales_prospects(id) ON DELETE SET NULL,
  company_id UUID REFERENCES sales_companies(id) ON DELETE SET NULL,
  contact_id UUID REFERENCES sales_contacts(id) ON DELETE SET NULL,
  
  -- Deal info
  stage VARCHAR(50) DEFAULT 'lead', -- 'lead', 'qualified', 'demo_scheduled', 'trial_started', 'negotiation', 'closed_won', 'closed_lost'
  
  -- Value estimation
  estimated_mrr_cents INTEGER, -- Estimated monthly revenue
  estimated_close_date DATE,
  
  -- Attribution
  source VARCHAR(100), -- 'outbound_email', 'inbound_signup', 'referral', 'organic'
  first_touch_channel VARCHAR(100),
  last_touch_channel VARCHAR(100),
  
  -- Status
  closed_at TIMESTAMP,
  closed_reason VARCHAR(255),
  
  -- Customer (if converted)
  customer_id UUID REFERENCES publishers(id) ON DELETE SET NULL,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_deals_stage ON sales_deals(stage);
CREATE INDEX IF NOT EXISTS idx_deals_prospect ON sales_deals(prospect_id);
CREATE INDEX IF NOT EXISTS idx_deals_company ON sales_deals(company_id);

-- Activity log (all sales activities)
CREATE TABLE IF NOT EXISTS sales_activities (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Related entities
  prospect_id UUID REFERENCES sales_prospects(id) ON DELETE SET NULL,
  company_id UUID REFERENCES sales_companies(id) ON DELETE SET NULL,
  contact_id UUID REFERENCES sales_contacts(id) ON DELETE SET NULL,
  deal_id UUID REFERENCES sales_deals(id) ON DELETE SET NULL,
  
  -- Activity details
  activity_type VARCHAR(50) NOT NULL, -- 'email_sent', 'email_opened', 'email_clicked', 'email_replied', 'signup', 'demo_request', 'trial_start', 'converted'
  channel VARCHAR(50), -- 'email', 'linkedin', 'phone', 'website'
  description TEXT,
  metadata JSONB DEFAULT '{}',
  
  -- Automated vs manual
  is_automated BOOLEAN DEFAULT true,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_activities_prospect ON sales_activities(prospect_id);
CREATE INDEX IF NOT EXISTS idx_activities_contact ON sales_activities(contact_id);
CREATE INDEX IF NOT EXISTS idx_activities_type ON sales_activities(activity_type);
CREATE INDEX IF NOT EXISTS idx_activities_created ON sales_activities(created_at DESC);

-- ============================================================================
-- ANALYTICS & OPTIMIZATION
-- ============================================================================

-- Daily sales metrics
CREATE TABLE IF NOT EXISTS sales_metrics_daily (
  date DATE NOT NULL,
  
  -- Discovery
  prospects_discovered INTEGER DEFAULT 0,
  prospects_enriched INTEGER DEFAULT 0,
  prospects_qualified INTEGER DEFAULT 0,
  prospects_disqualified INTEGER DEFAULT 0,
  
  -- Contacts
  contacts_discovered INTEGER DEFAULT 0,
  contacts_verified INTEGER DEFAULT 0,
  
  -- Outreach
  emails_scheduled INTEGER DEFAULT 0,
  emails_sent INTEGER DEFAULT 0,
  emails_delivered INTEGER DEFAULT 0,
  emails_opened INTEGER DEFAULT 0,
  emails_clicked INTEGER DEFAULT 0,
  emails_replied INTEGER DEFAULT 0,
  emails_bounced INTEGER DEFAULT 0,
  
  -- Pipeline
  deals_created INTEGER DEFAULT 0,
  deals_qualified INTEGER DEFAULT 0,
  demos_scheduled INTEGER DEFAULT 0,
  trials_started INTEGER DEFAULT 0,
  deals_won INTEGER DEFAULT 0,
  deals_lost INTEGER DEFAULT 0,
  
  -- Revenue
  pipeline_value_cents BIGINT DEFAULT 0,
  won_value_cents BIGINT DEFAULT 0,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  PRIMARY KEY (date)
);

-- Sequence performance (for A/B testing)
CREATE TABLE IF NOT EXISTS outreach_sequence_stats (
  sequence_id UUID NOT NULL REFERENCES outreach_sequences(id) ON DELETE CASCADE,
  step_number INTEGER NOT NULL,
  ab_variant VARCHAR(1),
  
  date DATE NOT NULL,
  
  -- Metrics
  sent INTEGER DEFAULT 0,
  delivered INTEGER DEFAULT 0,
  opened INTEGER DEFAULT 0,
  clicked INTEGER DEFAULT 0,
  replied INTEGER DEFAULT 0,
  bounced INTEGER DEFAULT 0,
  opted_out INTEGER DEFAULT 0,
  
  PRIMARY KEY (sequence_id, step_number, ab_variant, date)
);

-- ============================================================================
-- HELPER FUNCTIONS
-- ============================================================================

-- Function to calculate prospect fit score
CREATE OR REPLACE FUNCTION calculate_prospect_fit_score(p_id UUID) RETURNS INTEGER AS $$
DECLARE
  score INTEGER := 0;
  p RECORD;
BEGIN
  SELECT * INTO p FROM sales_prospects WHERE id = p_id;
  IF NOT FOUND THEN RETURN 0; END IF;
  
  -- Downloads (0-25 points)
  score := score + CASE p.downloads_estimate
    WHEN '5M-10M' THEN 25
    WHEN '1M-5M' THEN 22
    WHEN '500K-1M' THEN 18
    WHEN '100K-500K' THEN 15
    WHEN '50K-100K' THEN 10
    WHEN '10K-50K' THEN 5
    ELSE 0
  END;
  
  -- Has ads (0-20 points)
  IF p.has_ads THEN score := score + 20; END IF;
  
  -- Category (0-20 points)
  score := score + CASE p.app_category
    WHEN 'Games' THEN 20
    WHEN 'Entertainment' THEN 18
    WHEN 'Lifestyle' THEN 15
    WHEN 'Tools' THEN 12
    WHEN 'Education' THEN 10
    ELSE 5
  END;
  
  -- Rating (0-15 points)
  IF p.rating >= 4.5 THEN score := score + 15;
  ELSIF p.rating >= 4.0 THEN score := score + 12;
  ELSIF p.rating >= 3.5 THEN score := score + 8;
  ELSIF p.rating >= 3.0 THEN score := score + 4;
  END IF;
  
  -- Recent update (0-10 points)
  IF p.last_update_date > CURRENT_DATE - INTERVAL '30 days' THEN score := score + 10;
  ELSIF p.last_update_date > CURRENT_DATE - INTERVAL '90 days' THEN score := score + 5;
  END IF;
  
  -- Review count (0-10 points)
  IF p.review_count > 10000 THEN score := score + 10;
  ELSIF p.review_count > 1000 THEN score := score + 7;
  ELSIF p.review_count > 100 THEN score := score + 4;
  END IF;
  
  RETURN LEAST(score, 100);
END;
$$ LANGUAGE plpgsql;

-- Function to get next prospects to enrich
CREATE OR REPLACE FUNCTION get_prospects_to_enrich(batch_size INTEGER DEFAULT 50) 
RETURNS TABLE (id UUID, app_store VARCHAR, app_id VARCHAR, developer_name VARCHAR, company_domain VARCHAR) AS $$
BEGIN
  RETURN QUERY
  SELECT p.id, p.app_store, p.app_id, p.developer_name, p.company_domain
  FROM sales_prospects p
  WHERE p.status = 'discovered'
  ORDER BY 
    -- Prioritize by estimated value
    CASE p.downloads_estimate
      WHEN '5M-10M' THEN 1
      WHEN '1M-5M' THEN 2
      WHEN '500K-1M' THEN 3
      WHEN '100K-500K' THEN 4
      ELSE 5
    END,
    p.discovery_date DESC
  LIMIT batch_size;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- DEFAULT OUTREACH SEQUENCES
-- ============================================================================

-- Insert default cold outreach sequence
INSERT INTO outreach_sequences (id, name, description, target_segment, total_steps, step_delays_days, active)
VALUES (
  'a0000001-0000-0000-0000-000000000001',
  'Gaming Publishers - Cold Outreach',
  'Initial outreach to gaming app publishers with ads',
  'gaming',
  4,
  '{0, 3, 7, 14}',
  true
) ON CONFLICT DO NOTHING;

-- Step 1: Initial outreach
INSERT INTO outreach_sequence_steps (id, sequence_id, step_number, delay_days, subject_template, body_template)
VALUES (
  'b0000001-0000-0000-0000-000000000001',
  'a0000001-0000-0000-0000-000000000001',
  1,
  0,
  'Quick question about {{app_name}}''s ad monetization',
  '<!DOCTYPE html>
<html>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
<p>Hi {{first_name}},</p>

<p>I noticed {{app_name}} has grown to {{downloads}} downloads - congratulations on the traction!</p>

<p>Quick question: How satisfied are you with your current ad mediation setup?</p>

<p>I ask because we built <a href="https://apexmediation.ee">ApexMediation</a> specifically for publishers like you who want:</p>
<ul>
  <li><strong>Full transparency</strong> - See exactly why each ad won</li>
  <li><strong>Your own network accounts</strong> - No middleman taking a cut</li>
  <li><strong>Cryptographic proof</strong> - Verify no bid was suppressed</li>
</ul>

<p>We''re offering a <strong>free migration</strong> from {{current_mediation}} (no credit card, no commitment).</p>

<p>Would a quick 15-minute call next week work to see if we''re a fit?</p>

<p>Best,<br>
Aaron</p>

<p style="font-size: 12px; color: #666;">
ApexMediation<br>
<a href="https://apexmediation.ee">apexmediation.ee</a>
</p>
</body>
</html>'
) ON CONFLICT DO NOTHING;

-- Step 2: Value follow-up
INSERT INTO outreach_sequence_steps (id, sequence_id, step_number, delay_days, subject_template, body_template)
VALUES (
  'b0000002-0000-0000-0000-000000000001',
  'a0000001-0000-0000-0000-000000000001',
  2,
  3,
  'Re: Quick question about {{app_name}}''s ad monetization',
  '<!DOCTYPE html>
<html>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
<p>Hi {{first_name}},</p>

<p>Following up on my previous email. I wanted to share a quick insight:</p>

<p><strong>Publishers in the {{app_category}} category using transparent mediation see 12-18% higher eCPMs</strong> because they can actually optimize their waterfall based on real data.</p>

<p>Here''s what one of our publishers said:</p>

<blockquote style="border-left: 3px solid #2563eb; padding-left: 15px; margin: 15px 0; font-style: italic;">
"We switched from ironSource and finally understand why certain networks were being favored. Our revenue is up 15% in the first month."
</blockquote>

<p>I''d love to show you how this could work for {{app_name}}.</p>

<p>Got 15 minutes this week?</p>

<p>Best,<br>
Aaron</p>
</body>
</html>'
) ON CONFLICT DO NOTHING;

-- Step 3: Social proof
INSERT INTO outreach_sequence_steps (id, sequence_id, step_number, delay_days, subject_template, body_template)
VALUES (
  'b0000003-0000-0000-0000-000000000001',
  'a0000001-0000-0000-0000-000000000001',
  3,
  7,
  '{{app_name}} + ApexMediation ROI calculator',
  '<!DOCTYPE html>
<html>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
<p>Hi {{first_name}},</p>

<p>Based on {{app_name}}''s {{downloads}} downloads and typical {{app_category}} eCPMs, here''s what switching to transparent mediation could mean:</p>

<table style="border-collapse: collapse; margin: 20px 0;">
<tr>
  <td style="padding: 10px; border: 1px solid #ddd;"><strong>Current estimated ad revenue</strong></td>
  <td style="padding: 10px; border: 1px solid #ddd;">{{estimated_revenue}}/month</td>
</tr>
<tr>
  <td style="padding: 10px; border: 1px solid #ddd;"><strong>With transparency optimization</strong></td>
  <td style="padding: 10px; border: 1px solid #ddd;">+12-18% improvement potential</td>
</tr>
<tr>
  <td style="padding: 10px; border: 1px solid #ddd;"><strong>ApexMediation fee</strong></td>
  <td style="padding: 10px; border: 1px solid #ddd;">2.5% (Growth tier)</td>
</tr>
</table>

<p><strong>Net benefit:</strong> Even at conservative estimates, you''d be ahead by {{net_benefit}}/month.</p>

<p>Plus, your first $10K/app/month is free on our Starter tier.</p>

<p>Worth a quick chat?</p>

<p>Best,<br>
Aaron</p>
</body>
</html>'
) ON CONFLICT DO NOTHING;

-- Step 4: Breakup email
INSERT INTO outreach_sequence_steps (id, sequence_id, step_number, delay_days, subject_template, body_template)
VALUES (
  'b0000004-0000-0000-0000-000000000001',
  'a0000001-0000-0000-0000-000000000001',
  4,
  14,
  'Closing the loop on {{app_name}}',
  '<!DOCTYPE html>
<html>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
<p>Hi {{first_name}},</p>

<p>I''ve reached out a few times about improving {{app_name}}''s ad monetization, but I haven''t heard back.</p>

<p>No worries at all - I know you''re busy shipping features and growing your app.</p>

<p>I''ll assume the timing isn''t right. If things change and you want to explore transparent mediation, I''m here:</p>

<ul>
  <li>Free migration from {{current_mediation}}</li>
  <li>14-day trial, no credit card required</li>
  <li>Direct line to us: sales@apexmediation.ee</li>
</ul>

<p>Wishing {{app_name}} continued success! 🚀</p>

<p>Best,<br>
Aaron</p>

<p style="font-size: 11px; color: #999;">
P.S. If you''d rather not hear from us, just reply "unsubscribe" and we''ll remove you immediately.
</p>
</body>
</html>'
) ON CONFLICT DO NOTHING;

-- Create indexes for common queries
CREATE INDEX IF NOT EXISTS idx_outreach_next_send 
ON outreach_enrollments(next_step_at) 
WHERE status = 'active' AND next_step_at IS NOT NULL;

COMMENT ON TABLE sales_prospects IS 'Discovered potential customers (apps/companies) from public sources';
COMMENT ON TABLE sales_contacts IS 'Contact information for people at prospect companies';
COMMENT ON TABLE outreach_sequences IS 'Email sequence templates for automated outreach';
COMMENT ON TABLE outreach_enrollments IS 'Contacts enrolled in outreach sequences';
COMMENT ON TABLE outreach_emails IS 'Individual emails sent as part of sequences';
COMMENT ON TABLE sales_deals IS 'Sales pipeline tracking';
