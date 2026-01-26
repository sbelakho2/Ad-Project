-- Sales Automation CLI Tables
-- Run with: psql $DATABASE_URL -f setup.sql

-- Core tables for sales automation (minimal viable)
CREATE TABLE IF NOT EXISTS sa_contacts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  email VARCHAR(255) UNIQUE NOT NULL,
  first_name VARCHAR(100),
  last_name VARCHAR(100),
  company_name VARCHAR(255),
  job_title VARCHAR(255),
  app_name VARCHAR(255),
  app_downloads VARCHAR(50),
  status VARCHAR(30) DEFAULT 'new',
  source VARCHAR(100),
  notes TEXT,
  score INTEGER DEFAULT 50,
  email_verified BOOLEAN DEFAULT false,
  email_verification_status VARCHAR(30) DEFAULT 'unverified',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_sequences (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name VARCHAR(255) NOT NULL,
  description TEXT,
  active BOOLEAN DEFAULT true,
  steps JSONB NOT NULL DEFAULT '[]',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_enrollments (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  sequence_id UUID REFERENCES sa_sequences(id) ON DELETE CASCADE,
  status VARCHAR(30) DEFAULT 'active',
  current_step INTEGER DEFAULT 0,
  next_send_at TIMESTAMP,
  personalization JSONB DEFAULT '{}',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(contact_id, sequence_id)
);

CREATE TABLE IF NOT EXISTS sa_emails (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  enrollment_id UUID REFERENCES sa_enrollments(id) ON DELETE CASCADE,
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  to_email VARCHAR(255) NOT NULL,
  from_email VARCHAR(255) NOT NULL,
  subject TEXT NOT NULL,
  body_html TEXT NOT NULL,
  message_id VARCHAR(255),
  status VARCHAR(30) DEFAULT 'pending',
  step_number INTEGER,
  scheduled_for TIMESTAMP,
  sent_at TIMESTAMP,
  opened_at TIMESTAMP,
  clicked_at TIMESTAMP,
  replied_at TIMESTAMP,
  bounced_at TIMESTAMP,
  approved_at TIMESTAMP,
  approved_by VARCHAR(255),
  rejection_reason TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_activity_log (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  action VARCHAR(100) NOT NULL,
  details JSONB DEFAULT '{}',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Configuration table (editable from UI)
CREATE TABLE IF NOT EXISTS sa_config (
  key VARCHAR(100) PRIMARY KEY,
  value TEXT NOT NULL,
  description TEXT,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_by VARCHAR(255)
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_sa_contacts_email ON sa_contacts(email);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_status ON sa_contacts(status);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_verified ON sa_contacts(email_verified);
CREATE INDEX IF NOT EXISTS idx_sa_enrollments_next_send ON sa_enrollments(next_send_at) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_sa_emails_status ON sa_emails(status);

-- Default configuration values
INSERT INTO sa_config (key, value, description) VALUES
  ('from_email', 'sales@apexmediation.ee', 'Default sender email address'),
  ('from_name', 'Aaron from ApexMediation', 'Default sender display name'),
  ('reply_to', 'sales@apexmediation.ee', 'Reply-to email address'),
  ('daily_limit', '100', 'Maximum emails to send per day'),
  ('require_email_verification', 'false', 'Require email verification before enrolling contacts'),
  ('enable_review_queue', 'false', 'Create drafts instead of sending immediately'),
  ('warmup_mode', 'false', 'Enable gradual sending increase for new domains'),
  ('warmup_daily_increment', '5', 'Number of additional emails per day during warmup'),
  ('system_enabled', 'false', 'Master switch for sales automation'),
  ('spam_score_threshold', '70', 'Minimum spam score (0-100) to allow sending'),
  ('bounce_rate_threshold', '7', 'Maximum bounce rate (%) before auto-pause'),
  ('pause_on_critical_health', 'true', 'Automatically pause when domain health is critical')
ON CONFLICT (key) DO NOTHING;

-- Default sequences (FIXED Jan 2026: Use only variables guaranteed in contact data)
-- Variables available: first_name, last_name, company_name, email, job_title
-- Do NOT use: app_name, app_downloads (unreliable data coverage)
INSERT INTO sa_sequences (name, description, steps) 
SELECT 'Mobile App Publisher - Ad Mediation', 'For mobile app developers who show ads', 
'[{"delay_days": 0, "subject": "Quick question for {{company_name}}", "body": "Hi {{first_name}},\n\nI came across {{company_name}} and noticed you work with mobile apps.\n\nQuick question - are you using a single ad network or mediation for monetization?\n\nMany publishers we work with find they can improve fill rates and eCPM by optimizing their demand stack.\n\nWould you be open to a 15-minute call to see if this could help?\n\nBest,\nAaron\n\nP.S. Reply NO to opt out - I won'\''t follow up."}, {"delay_days": 3, "subject": "Following up - {{company_name}}", "body": "Hi {{first_name}},\n\nFollowing up on my last email.\n\nI understand you'\''re busy - just wanted to share that similar publishers often see meaningful improvements after optimizing their ad stack.\n\nWorth a quick chat?\n\nBest,\nAaron\n\nP.S. Reply NO to opt out."}, {"delay_days": 5, "subject": "Last note - {{company_name}}", "body": "Hi {{first_name}},\n\nLast email from me!\n\nIf ad monetization isn'\''t a priority right now, I totally understand. But if you'\''re ever curious about how other publishers optimize their stack, happy to share insights.\n\nEither way, wishing you continued success.\n\nCheers,\nAaron\n\nP.S. Reply NO to opt out."}]'::jsonb
WHERE NOT EXISTS (SELECT 1 FROM sa_sequences WHERE name = 'Mobile App Publisher - Ad Mediation');

INSERT INTO sa_sequences (name, description, steps) 
SELECT 'Simple Introduction', 'Basic intro sequence for general outreach',
'[{"delay_days": 0, "subject": "Quick question for {{company_name}}", "body": "Hi {{first_name}},\n\nI came across {{company_name}} and wanted to reach out.\n\nWe help companies like yours improve their ad revenue through better demand optimization.\n\nWould you be open to a brief call this week?\n\nBest,\nAaron\n\nP.S. Reply NO to opt out - I won'\''t follow up."}, {"delay_days": 4, "subject": "Following up - {{company_name}}", "body": "Hi {{first_name}},\n\nJust following up on my previous email.\n\nI know you'\''re busy, so I'\''ll keep it short: we'\''ve helped similar companies see meaningful improvements with minimal effort on their end.\n\n5 minutes to chat?\n\nAaron\n\nP.S. Reply NO to opt out."}]'::jsonb
WHERE NOT EXISTS (SELECT 1 FROM sa_sequences WHERE name = 'Simple Introduction');
