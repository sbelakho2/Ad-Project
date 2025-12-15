-- Control Plane: Operator trusted devices + device approval challenges
-- Purpose: Email-confirm new devices; once approved, device is whitelisted.

CREATE SCHEMA IF NOT EXISTS control_plane;

CREATE TABLE IF NOT EXISTS control_plane.operator_trusted_devices (
  id BIGSERIAL PRIMARY KEY,
  operator_email TEXT NOT NULL,
  device_hash TEXT NOT NULL,
  device_label TEXT,
  user_agent TEXT,
  added_via TEXT NOT NULL DEFAULT 'bootstrap',
  added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  revoked_at TIMESTAMPTZ,
  revoked_by TEXT,
  revoked_reason TEXT,

  CONSTRAINT operator_trusted_devices_email_chk CHECK (operator_email <> ''),
  CONSTRAINT operator_trusted_devices_device_hash_chk CHECK (device_hash <> ''),
  CONSTRAINT operator_trusted_devices_added_via_chk CHECK (added_via IN ('bootstrap','email'))
);

CREATE UNIQUE INDEX IF NOT EXISTS operator_trusted_devices_uniq
  ON control_plane.operator_trusted_devices (operator_email, device_hash)
  WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS operator_trusted_devices_email_idx
  ON control_plane.operator_trusted_devices (operator_email);

CREATE TABLE IF NOT EXISTS control_plane.operator_device_challenges (
  id BIGSERIAL PRIMARY KEY,
  operator_email TEXT NOT NULL,
  device_hash TEXT NOT NULL,
  device_label TEXT,
  user_agent TEXT,
  token_hash TEXT NOT NULL,
  requested_ip TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,

  CONSTRAINT operator_device_challenges_email_chk CHECK (operator_email <> ''),
  CONSTRAINT operator_device_challenges_device_hash_chk CHECK (device_hash <> ''),
  CONSTRAINT operator_device_challenges_token_hash_chk CHECK (token_hash <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS operator_device_challenges_token_uniq
  ON control_plane.operator_device_challenges (token_hash);

CREATE INDEX IF NOT EXISTS operator_device_challenges_lookup_idx
  ON control_plane.operator_device_challenges (operator_email, device_hash, created_at);

-- Grants
GRANT USAGE ON SCHEMA control_plane TO apex_readonly, apex_operator, apex_breakglass;

GRANT SELECT ON control_plane.operator_trusted_devices TO apex_readonly;
GRANT SELECT ON control_plane.operator_device_challenges TO apex_readonly;

GRANT SELECT, INSERT, UPDATE, DELETE ON control_plane.operator_trusted_devices TO apex_operator;
GRANT SELECT, INSERT, UPDATE, DELETE ON control_plane.operator_device_challenges TO apex_operator;

GRANT SELECT ON control_plane.operator_trusted_devices TO apex_breakglass;
GRANT SELECT ON control_plane.operator_device_challenges TO apex_breakglass;

ALTER DEFAULT PRIVILEGES IN SCHEMA control_plane GRANT SELECT ON TABLES TO apex_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA control_plane GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO apex_operator;
ALTER DEFAULT PRIVILEGES IN SCHEMA control_plane GRANT SELECT ON TABLES TO apex_breakglass;
