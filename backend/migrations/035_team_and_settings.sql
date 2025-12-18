-- Team Management Tables
-- Supports team member invitations, role-based access, and multi-user organizations

-- Team members table
CREATE TABLE IF NOT EXISTS team_members (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  publisher_id UUID NOT NULL REFERENCES publishers(id) ON DELETE CASCADE,
  user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  email VARCHAR(255) NOT NULL,
  role VARCHAR(50) NOT NULL DEFAULT 'developer',
  status VARCHAR(50) NOT NULL DEFAULT 'invited',
  invite_token UUID,
  permissions JSONB DEFAULT '[]',
  created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  CONSTRAINT team_members_publisher_email_unique UNIQUE (publisher_id, email)
);

-- Publisher settings table (for fraud, notifications, compliance settings)
CREATE TABLE IF NOT EXISTS publisher_settings (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  publisher_id UUID NOT NULL REFERENCES publishers(id) ON DELETE CASCADE,
  category VARCHAR(50) NOT NULL, -- 'fraud', 'notifications', 'compliance'
  settings JSONB NOT NULL DEFAULT '{}',
  created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  CONSTRAINT publisher_settings_unique UNIQUE (publisher_id, category)
);

-- Indexes for faster lookups
CREATE INDEX IF NOT EXISTS idx_team_members_publisher ON team_members(publisher_id);
CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_members(user_id);
CREATE INDEX IF NOT EXISTS idx_team_members_invite_token ON team_members(invite_token) WHERE invite_token IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_team_members_status ON team_members(status);
CREATE INDEX IF NOT EXISTS idx_publisher_settings_publisher ON publisher_settings(publisher_id);
CREATE INDEX IF NOT EXISTS idx_publisher_settings_category ON publisher_settings(category);

-- Comments
COMMENT ON TABLE team_members IS 'Team members and pending invitations for each publisher';
COMMENT ON TABLE publisher_settings IS 'Publisher-specific settings for fraud protection, notifications, and compliance';
