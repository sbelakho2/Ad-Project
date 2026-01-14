-- Two-Factor Authentication Table
-- Created: 2025-12-23
-- Purpose: Store 2FA configuration for user accounts
--
-- This table was missing from initial migrations and caused login failures
-- when the auth controller checked for 2FA status.

CREATE TABLE IF NOT EXISTS two_factor_auth (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  secret TEXT,                    -- Deprecated: plaintext secret (kept nullable for backward compatibility)
  secret_ciphertext TEXT,         -- Encrypted TOTP secret (AES-GCM base64 payload as JSON string)
  enabled BOOLEAN DEFAULT FALSE,  -- Whether 2FA is enabled for this user
  backup_codes TEXT               -- Comma-separated hashed backup codes
);

CREATE INDEX IF NOT EXISTS idx_two_factor_auth_user_enabled 
  ON two_factor_auth(user_id, enabled);

-- Add comment for documentation
COMMENT ON TABLE two_factor_auth IS 'Two-factor authentication settings per user';
COMMENT ON COLUMN two_factor_auth.secret IS 'Deprecated plaintext TOTP secret (use secret_ciphertext instead)';
COMMENT ON COLUMN two_factor_auth.secret_ciphertext IS 'Encrypted TOTP secret using AES-GCM';
COMMENT ON COLUMN two_factor_auth.backup_codes IS 'Hashed backup codes for account recovery';
