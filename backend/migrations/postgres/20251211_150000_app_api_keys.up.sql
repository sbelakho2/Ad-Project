-- App-scoped SDK/API keys
-- Each publisher can have multiple apps; each app can have multiple keys (rotation).

CREATE TABLE IF NOT EXISTS app_api_keys (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  publisher_id UUID NOT NULL REFERENCES publishers(id) ON DELETE CASCADE,
  app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  environment TEXT NOT NULL DEFAULT 'production',

  -- bcrypt hash of the secret
  secret TEXT NOT NULL,

  -- sha256(secret) hex for constant-time lookup
  secret_digest TEXT,

  prefix TEXT NOT NULL,
  last4 TEXT NOT NULL,

  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_used_at TIMESTAMPTZ,
  revoked_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_app_api_keys_secret_digest
  ON app_api_keys(secret_digest)
  WHERE secret_digest IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_app_api_keys_publisher_app
  ON app_api_keys(publisher_id, app_id);

CREATE INDEX IF NOT EXISTS idx_app_api_keys_app_active
  ON app_api_keys(app_id, revoked_at);

-- Minimal usage/audit trail (IP/UA hashed)
CREATE TABLE IF NOT EXISTS app_api_key_usages (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  app_api_key_id UUID REFERENCES app_api_keys(id) ON DELETE CASCADE,
  ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  route VARCHAR(128),
  method VARCHAR(64),
  ip_hash VARCHAR(64),
  ua_hash VARCHAR(64),
  status VARCHAR(16)
);

CREATE INDEX IF NOT EXISTS idx_app_api_key_usages_key
  ON app_api_key_usages(app_api_key_id);
