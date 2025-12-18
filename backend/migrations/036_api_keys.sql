-- Migration: Create API Keys table for TypeORM
-- Required by: apiKeys.controller.ts
-- NOTE: Column names match TypeORM default behavior (camelCase → lowercase)

CREATE TABLE IF NOT EXISTS api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    "userId" UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    secret TEXT NOT NULL,
    "secretDigest" TEXT UNIQUE,
    prefix VARCHAR(20) NOT NULL,
    last4 VARCHAR(4) NOT NULL,
    "createdAt" TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    "updatedAt" TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    "lastUsedAt" TIMESTAMP WITH TIME ZONE,
    "revokedAt" TIMESTAMP WITH TIME ZONE
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_api_keys_user ON api_keys("userId");
CREATE INDEX IF NOT EXISTS idx_api_keys_secret_digest ON api_keys("secretDigest");
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(prefix);

-- Record migration
INSERT INTO schema_migrations (version, name, applied_at)
VALUES ('036', 'api_keys', NOW())
ON CONFLICT (version) DO NOTHING;
