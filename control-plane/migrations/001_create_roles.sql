-- Control Plane Database Roles Migration
-- Run this as the postgres superuser BEFORE deploying the control plane
--
-- This creates three separate database roles with graduated privileges:
-- 1. apex_readonly - for monitoring/telemetry (read-only)
-- 2. apex_operator - for safe actions (partition management)
-- 3. apex_breakglass - for emergency actions (session termination)

-- ============================================
-- READ-ONLY ROLE (Monitoring/Telemetry)
-- ============================================
DO $$ 
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'apex_readonly') THEN
    CREATE ROLE apex_readonly WITH LOGIN PASSWORD 'CHANGE_ME_readonly_password';
  END IF;
END $$;

-- Grant basic access
GRANT CONNECT ON DATABASE apexmediation TO apex_readonly;
GRANT USAGE ON SCHEMA public TO apex_readonly;

-- Grant SELECT on all existing tables
GRANT SELECT ON ALL TABLES IN SCHEMA public TO apex_readonly;

-- Grant SELECT on future tables
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO apex_readonly;

-- Grant access to pg_stat views for monitoring
GRANT pg_read_all_stats TO apex_readonly;

-- ============================================
-- OPERATOR ROLE (Partition Management)
-- ============================================
DO $$ 
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'apex_operator') THEN
    CREATE ROLE apex_operator WITH LOGIN PASSWORD 'CHANGE_ME_operator_password';
  END IF;
END $$;

-- Grant basic access
GRANT CONNECT ON DATABASE apexmediation TO apex_operator;
GRANT USAGE ON SCHEMA public TO apex_operator;

-- Grant read access like readonly
GRANT SELECT ON ALL TABLES IN SCHEMA public TO apex_operator;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO apex_operator;
GRANT pg_read_all_stats TO apex_operator;

-- Grant partition management (CREATE for new partition tables)
GRANT CREATE ON SCHEMA public TO apex_operator;

-- Grant write access to specific tables
GRANT SELECT, INSERT, UPDATE ON system_state TO apex_operator;
GRANT SELECT, INSERT ON audit_logs TO apex_operator;

-- ============================================
-- BREAK-GLASS ROLE (Emergency Actions)
-- ============================================
DO $$ 
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'apex_breakglass') THEN
    CREATE ROLE apex_breakglass WITH LOGIN PASSWORD 'CHANGE_ME_breakglass_password';
  END IF;
END $$;

-- Grant basic access
GRANT CONNECT ON DATABASE apexmediation TO apex_breakglass;
GRANT USAGE ON SCHEMA public TO apex_breakglass;

-- Grant session termination capability
GRANT pg_signal_backend TO apex_breakglass;

-- Grant read access for verification
GRANT SELECT ON pg_stat_activity TO apex_breakglass;

-- Grant write to audit log
GRANT SELECT, INSERT ON audit_logs TO apex_breakglass;

-- ============================================
-- SYSTEM STATE TABLE (for mode flags)
-- ============================================
CREATE TABLE IF NOT EXISTS system_state (
  key VARCHAR(255) PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  updated_by VARCHAR(255)
);

-- Default system state
INSERT INTO system_state (key, value, updated_at, updated_by)
VALUES ('mode', 'OK', NOW(), 'migration')
ON CONFLICT (key) DO NOTHING;

-- ============================================
-- AUDIT LOGS TABLE (for control plane actions)
-- ============================================
CREATE TABLE IF NOT EXISTS audit_logs (
  id SERIAL PRIMARY KEY,
  timestamp TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  actor VARCHAR(255) NOT NULL,
  action VARCHAR(255) NOT NULL,
  target VARCHAR(255),
  pre_state JSONB,
  post_state JSONB,
  result VARCHAR(50) NOT NULL,
  reason TEXT,
  ip_address VARCHAR(50),
  user_agent TEXT,
  
  -- Indexes for querying
  CONSTRAINT audit_logs_result_check CHECK (result IN ('success', 'failure', 'denied'))
);

-- Index for common queries
CREATE INDEX IF NOT EXISTS idx_audit_logs_timestamp ON audit_logs(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_actor ON audit_logs(actor);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON audit_logs(action);

-- Grant access to audit_logs
GRANT SELECT, INSERT ON audit_logs TO apex_readonly;
GRANT SELECT, INSERT ON audit_logs TO apex_operator;
GRANT SELECT, INSERT ON audit_logs TO apex_breakglass;
GRANT USAGE, SELECT ON SEQUENCE audit_logs_id_seq TO apex_readonly;
GRANT USAGE, SELECT ON SEQUENCE audit_logs_id_seq TO apex_operator;
GRANT USAGE, SELECT ON SEQUENCE audit_logs_id_seq TO apex_breakglass;

-- ============================================
-- VERIFICATION QUERIES
-- ============================================
-- Run these to verify the roles are set up correctly:
--
-- SELECT rolname, rolcanlogin, rolsuper 
-- FROM pg_roles 
-- WHERE rolname LIKE 'apex_%';
--
-- SELECT grantee, privilege_type, table_name 
-- FROM information_schema.role_table_grants 
-- WHERE grantee LIKE 'apex_%';

-- ============================================
-- PASSWORD CHANGE REMINDER
-- ============================================
-- IMPORTANT: Change the placeholder passwords above!
-- Use strong, unique passwords for each role.
-- Store passwords securely in environment variables.
--
-- Example commands to change passwords:
-- ALTER ROLE apex_readonly PASSWORD 'new_secure_password';
-- ALTER ROLE apex_operator PASSWORD 'new_secure_password';
-- ALTER ROLE apex_breakglass PASSWORD 'new_secure_password';
