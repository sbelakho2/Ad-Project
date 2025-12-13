-- Migration: 034_hetzner_schema_reorganization
-- Description: Reorganize database into 5 workload-specific schemas for Hetzner deployment
-- Date: 2025-12-13
-- Author: System
-- 
-- This migration creates the new schema structure:
--   - core: Business entities (publishers, apps, placements, adapters)
--   - config: Runtime configuration (feature flags, AB tests, adapter configs)
--   - ops: Operational data (autopilot, jobs, audit logs)
--   - events: High-volume event data (partitioned by day)
--   - agg: Pre-computed aggregates for dashboards

-- ============================================================================
-- PHASE 1: Create Schemas
-- ============================================================================

CREATE SCHEMA IF NOT EXISTS core;
CREATE SCHEMA IF NOT EXISTS config;
CREATE SCHEMA IF NOT EXISTS ops;
CREATE SCHEMA IF NOT EXISTS events;
CREATE SCHEMA IF NOT EXISTS agg;

-- Grant permissions to application role
DO $$
BEGIN
    -- Create role if not exists
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app_service') THEN
        CREATE ROLE app_service WITH LOGIN PASSWORD 'CHANGE_ME_IN_PRODUCTION';
    END IF;
    
    -- Grant schema usage
    GRANT USAGE ON SCHEMA core TO app_service;
    GRANT USAGE ON SCHEMA config TO app_service;
    GRANT USAGE ON SCHEMA ops TO app_service;
    GRANT USAGE ON SCHEMA events TO app_service;
    GRANT USAGE ON SCHEMA agg TO app_service;
    
    -- Grant table permissions
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA core TO app_service;
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA config TO app_service;
    GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA ops TO app_service;
    GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA events TO app_service;
    GRANT SELECT ON ALL TABLES IN SCHEMA agg TO app_service;
    
    -- Grant sequence permissions
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA core TO app_service;
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA config TO app_service;
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA ops TO app_service;
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA events TO app_service;
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA agg TO app_service;
    
    -- Default privileges for future tables
    ALTER DEFAULT PRIVILEGES IN SCHEMA core GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO app_service;
    ALTER DEFAULT PRIVILEGES IN SCHEMA config GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO app_service;
    ALTER DEFAULT PRIVILEGES IN SCHEMA ops GRANT SELECT, INSERT ON TABLES TO app_service;
    ALTER DEFAULT PRIVILEGES IN SCHEMA events GRANT SELECT, INSERT ON TABLES TO app_service;
    ALTER DEFAULT PRIVILEGES IN SCHEMA agg GRANT SELECT ON TABLES TO app_service;
END $$;

-- ============================================================================
-- PHASE 2: Core Schema - Business Entities
-- ============================================================================

-- Publishers table (moved from public schema)
CREATE TABLE IF NOT EXISTS core.publishers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    domain VARCHAR(255),
    status VARCHAR(50) DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'suspended', 'pending')),
    tier VARCHAR(50) DEFAULT 'standard' CHECK (tier IN ('standard', 'premium', 'enterprise')),
    settings JSONB DEFAULT '{}',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_publishers_status ON core.publishers(status);
CREATE INDEX IF NOT EXISTS idx_publishers_tier ON core.publishers(tier);
CREATE INDEX IF NOT EXISTS idx_publishers_email ON core.publishers(email);

-- Apps table
CREATE TABLE IF NOT EXISTS core.apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    bundle_id VARCHAR(255),
    platform VARCHAR(50) CHECK (platform IN ('ios', 'android', 'web', 'ctv')),
    status VARCHAR(50) DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'pending', 'rejected')),
    app_store_url TEXT,
    settings JSONB DEFAULT '{}',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(publisher_id, bundle_id)
);

CREATE INDEX IF NOT EXISTS idx_apps_publisher ON core.apps(publisher_id);
CREATE INDEX IF NOT EXISTS idx_apps_platform ON core.apps(platform);
CREATE INDEX IF NOT EXISTS idx_apps_status ON core.apps(status);
CREATE INDEX IF NOT EXISTS idx_apps_bundle ON core.apps(bundle_id);

-- Placements table
CREATE TABLE IF NOT EXISTS core.placements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES core.apps(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    placement_type VARCHAR(50) CHECK (placement_type IN ('banner', 'interstitial', 'rewarded', 'native', 'mrec')),
    status VARCHAR(50) DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'paused')),
    floor_price DECIMAL(10,4) DEFAULT 0,
    refresh_interval INTEGER DEFAULT 30,
    settings JSONB DEFAULT '{}',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_placements_app ON core.placements(app_id);
CREATE INDEX IF NOT EXISTS idx_placements_type ON core.placements(placement_type);
CREATE INDEX IF NOT EXISTS idx_placements_status ON core.placements(status);

-- Adapters table
CREATE TABLE IF NOT EXISTS core.adapters (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL UNIQUE,
    display_name VARCHAR(255) NOT NULL,
    type VARCHAR(50) CHECK (type IN ('bidding', 'waterfall', 'hybrid')),
    platform_support JSONB DEFAULT '["ios", "android", "web"]',
    sdk_version VARCHAR(50),
    status VARCHAR(50) DEFAULT 'active' CHECK (status IN ('active', 'deprecated', 'disabled')),
    default_config JSONB DEFAULT '{}',
    documentation_url TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_adapters_type ON core.adapters(type);
CREATE INDEX IF NOT EXISTS idx_adapters_status ON core.adapters(status);

-- Placement Adapters (many-to-many with config)
CREATE TABLE IF NOT EXISTS core.placement_adapters (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    placement_id UUID NOT NULL REFERENCES core.placements(id) ON DELETE CASCADE,
    adapter_id UUID NOT NULL REFERENCES core.adapters(id) ON DELETE CASCADE,
    priority INTEGER DEFAULT 0,
    enabled BOOLEAN DEFAULT true,
    config JSONB DEFAULT '{}',
    credentials JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(placement_id, adapter_id)
);

CREATE INDEX IF NOT EXISTS idx_placement_adapters_placement ON core.placement_adapters(placement_id);
CREATE INDEX IF NOT EXISTS idx_placement_adapters_adapter ON core.placement_adapters(adapter_id);

-- Users table
CREATE TABLE IF NOT EXISTS core.users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID REFERENCES core.publishers(id) ON DELETE CASCADE,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255),
    name VARCHAR(255),
    role VARCHAR(50) DEFAULT 'viewer' CHECK (role IN ('admin', 'owner', 'manager', 'viewer')),
    status VARCHAR(50) DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'pending', 'suspended')),
    last_login TIMESTAMP WITH TIME ZONE,
    mfa_enabled BOOLEAN DEFAULT false,
    mfa_secret VARCHAR(255),
    settings JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_users_publisher ON core.users(publisher_id);
CREATE INDEX IF NOT EXISTS idx_users_email ON core.users(email);
CREATE INDEX IF NOT EXISTS idx_users_role ON core.users(role);

-- VRA Ledger (append-only with hash chain)
CREATE TABLE IF NOT EXISTS core.vra_ledger (
    id BIGSERIAL,
    tenant_id UUID NOT NULL,
    event_type VARCHAR(50) NOT NULL,
    amount DECIMAL(15,6) NOT NULL,
    currency VARCHAR(3) DEFAULT 'USD',
    prev_hash VARCHAR(64),
    current_hash VARCHAR(64) NOT NULL,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_vra_ledger_tenant ON core.vra_ledger(tenant_id);
CREATE INDEX IF NOT EXISTS idx_vra_ledger_hash ON core.vra_ledger(current_hash);

-- ============================================================================
-- PHASE 3: Config Schema - Runtime Configuration
-- ============================================================================

-- Feature flags table
CREATE TABLE IF NOT EXISTS config.feature_flags (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL UNIQUE,
    description TEXT,
    enabled BOOLEAN DEFAULT false,
    rollout_percentage INTEGER DEFAULT 0 CHECK (rollout_percentage >= 0 AND rollout_percentage <= 100),
    targeting_rules JSONB DEFAULT '{}',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_feature_flags_name ON config.feature_flags(name);
CREATE INDEX IF NOT EXISTS idx_feature_flags_enabled ON config.feature_flags(enabled);

-- AB Tests table
CREATE TABLE IF NOT EXISTS config.ab_tests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    description TEXT,
    status VARCHAR(50) DEFAULT 'draft' CHECK (status IN ('draft', 'running', 'paused', 'completed', 'cancelled')),
    start_date TIMESTAMP WITH TIME ZONE,
    end_date TIMESTAMP WITH TIME ZONE,
    traffic_allocation DECIMAL(5,2) DEFAULT 100.00,
    variants JSONB NOT NULL DEFAULT '[]',
    targeting JSONB DEFAULT '{}',
    metrics JSONB DEFAULT '[]',
    results JSONB DEFAULT '{}',
    created_by UUID,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ab_tests_status ON config.ab_tests(status);
CREATE INDEX IF NOT EXISTS idx_ab_tests_dates ON config.ab_tests(start_date, end_date);

-- Adapter configurations (per-tenant overrides)
CREATE TABLE IF NOT EXISTS config.adapter_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID REFERENCES core.publishers(id) ON DELETE CASCADE,
    adapter_id UUID NOT NULL REFERENCES core.adapters(id) ON DELETE CASCADE,
    config JSONB NOT NULL DEFAULT '{}',
    credentials JSONB DEFAULT '{}',
    enabled BOOLEAN DEFAULT true,
    priority INTEGER DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(publisher_id, adapter_id)
);

CREATE INDEX IF NOT EXISTS idx_adapter_configs_publisher ON config.adapter_configs(publisher_id);
CREATE INDEX IF NOT EXISTS idx_adapter_configs_adapter ON config.adapter_configs(adapter_id);

-- Migration experiments
CREATE TABLE IF NOT EXISTS config.migration_experiments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    app_id UUID REFERENCES core.apps(id) ON DELETE SET NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    status VARCHAR(50) DEFAULT 'draft' CHECK (status IN ('draft', 'active', 'paused', 'completed', 'cancelled')),
    control_arm JSONB NOT NULL DEFAULT '{}',
    test_arm JSONB NOT NULL DEFAULT '{}',
    traffic_split INTEGER DEFAULT 50 CHECK (traffic_split >= 0 AND traffic_split <= 100),
    start_date TIMESTAMP WITH TIME ZONE,
    end_date TIMESTAMP WITH TIME ZONE,
    metrics JSONB DEFAULT '{}',
    created_by UUID,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_migration_experiments_publisher ON config.migration_experiments(publisher_id);
CREATE INDEX IF NOT EXISTS idx_migration_experiments_status ON config.migration_experiments(status);

-- SDK configurations
CREATE TABLE IF NOT EXISTS config.sdk_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES core.apps(id) ON DELETE CASCADE,
    platform VARCHAR(50) NOT NULL CHECK (platform IN ('ios', 'android', 'web', 'ctv')),
    version VARCHAR(50),
    config JSONB NOT NULL DEFAULT '{}',
    min_sdk_version VARCHAR(50),
    max_sdk_version VARCHAR(50),
    status VARCHAR(50) DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'deprecated')),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(app_id, platform, version)
);

CREATE INDEX IF NOT EXISTS idx_sdk_configs_app ON config.sdk_configs(app_id);
CREATE INDEX IF NOT EXISTS idx_sdk_configs_platform ON config.sdk_configs(platform);

-- ============================================================================
-- PHASE 4: Ops Schema - Operational Data
-- ============================================================================

-- Autopilot status (singleton per database)
CREATE TABLE IF NOT EXISTS ops.autopilot_status (
    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    state VARCHAR(20) NOT NULL DEFAULT 'OK' CHECK (state IN ('OK', 'DEGRADED', 'FAIL')),
    last_check TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    checks JSONB DEFAULT '{}',
    metrics JSONB DEFAULT '{}',
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Insert initial row
INSERT INTO ops.autopilot_status (id, state, checks, metrics) 
VALUES (1, 'OK', '{}', '{}')
ON CONFLICT (id) DO NOTHING;

-- Autopilot jobs (scheduled maintenance tasks)
CREATE TABLE IF NOT EXISTS ops.autopilot_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_type VARCHAR(100) NOT NULL,
    status VARCHAR(50) DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'skipped')),
    scheduled_at TIMESTAMP WITH TIME ZONE NOT NULL,
    started_at TIMESTAMP WITH TIME ZONE,
    completed_at TIMESTAMP WITH TIME ZONE,
    duration_ms INTEGER,
    result JSONB DEFAULT '{}',
    error TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_autopilot_jobs_type ON ops.autopilot_jobs(job_type);
CREATE INDEX IF NOT EXISTS idx_autopilot_jobs_status ON ops.autopilot_jobs(status);
CREATE INDEX IF NOT EXISTS idx_autopilot_jobs_scheduled ON ops.autopilot_jobs(scheduled_at);

-- Audit logs
CREATE TABLE IF NOT EXISTS ops.audit_logs (
    id BIGSERIAL,
    actor_id UUID,
    actor_type VARCHAR(50) CHECK (actor_type IN ('user', 'system', 'api', 'autopilot')),
    action VARCHAR(100) NOT NULL,
    resource_type VARCHAR(100) NOT NULL,
    resource_id UUID,
    old_value JSONB,
    new_value JSONB,
    ip_address INET,
    user_agent TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_audit_logs_actor ON ops.audit_logs(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON ops.audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource ON ops.audit_logs(resource_type, resource_id);

-- Billing audit trail
CREATE TABLE IF NOT EXISTS ops.billing_audit (
    id BIGSERIAL,
    publisher_id UUID NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    amount DECIMAL(15,6),
    currency VARCHAR(3) DEFAULT 'USD',
    stripe_event_id VARCHAR(255),
    invoice_id VARCHAR(255),
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_billing_audit_publisher ON ops.billing_audit(publisher_id);
CREATE INDEX IF NOT EXISTS idx_billing_audit_stripe ON ops.billing_audit(stripe_event_id);

-- Background jobs log
CREATE TABLE IF NOT EXISTS ops.job_logs (
    id BIGSERIAL,
    queue VARCHAR(100) NOT NULL,
    job_id VARCHAR(255) NOT NULL,
    job_type VARCHAR(100) NOT NULL,
    status VARCHAR(50) CHECK (status IN ('queued', 'running', 'completed', 'failed', 'retrying')),
    attempts INTEGER DEFAULT 0,
    max_attempts INTEGER DEFAULT 3,
    payload JSONB DEFAULT '{}',
    result JSONB,
    error TEXT,
    started_at TIMESTAMP WITH TIME ZONE,
    completed_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_job_logs_queue ON ops.job_logs(queue);
CREATE INDEX IF NOT EXISTS idx_job_logs_status ON ops.job_logs(status);
CREATE INDEX IF NOT EXISTS idx_job_logs_job ON ops.job_logs(job_id);

-- ============================================================================
-- PHASE 5: Events Schema - High-Volume Event Data (Partitioned)
-- ============================================================================

-- Ad requests (partitioned by day)
CREATE TABLE IF NOT EXISTS events.ad_requests (
    id UUID DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    device_id VARCHAR(255),
    session_id VARCHAR(255),
    request_type VARCHAR(50),
    platform VARCHAR(50),
    country VARCHAR(2),
    region VARCHAR(100),
    device_type VARCHAR(50),
    os_version VARCHAR(50),
    sdk_version VARCHAR(50),
    adapters_requested JSONB DEFAULT '[]',
    floor_price DECIMAL(10,4),
    timeout_ms INTEGER,
    latency_ms INTEGER,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_ad_requests_tenant ON events.ad_requests(tenant_id);
CREATE INDEX IF NOT EXISTS idx_ad_requests_app ON events.ad_requests(app_id);
CREATE INDEX IF NOT EXISTS idx_ad_requests_placement ON events.ad_requests(placement_id);
CREATE INDEX IF NOT EXISTS idx_ad_requests_country ON events.ad_requests(country);

-- Bid responses
CREATE TABLE IF NOT EXISTS events.bid_responses (
    id UUID DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    adapter_id UUID NOT NULL,
    bid_price DECIMAL(10,6),
    currency VARCHAR(3) DEFAULT 'USD',
    won BOOLEAN DEFAULT false,
    response_time_ms INTEGER,
    error_code VARCHAR(50),
    error_message TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_bid_responses_request ON events.bid_responses(request_id);
CREATE INDEX IF NOT EXISTS idx_bid_responses_adapter ON events.bid_responses(adapter_id);
CREATE INDEX IF NOT EXISTS idx_bid_responses_won ON events.bid_responses(won);

-- Impressions
CREATE TABLE IF NOT EXISTS events.impressions (
    id UUID DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID NOT NULL,
    request_id UUID,
    device_id VARCHAR(255),
    session_id VARCHAR(255),
    revenue DECIMAL(10,6),
    currency VARCHAR(3) DEFAULT 'USD',
    country VARCHAR(2),
    platform VARCHAR(50),
    ad_format VARCHAR(50),
    viewability_score DECIMAL(5,2),
    is_viewable BOOLEAN DEFAULT false,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_impressions_tenant ON events.impressions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_impressions_app ON events.impressions(app_id);
CREATE INDEX IF NOT EXISTS idx_impressions_adapter ON events.impressions(adapter_id);
CREATE INDEX IF NOT EXISTS idx_impressions_country ON events.impressions(country);

-- Clicks
CREATE TABLE IF NOT EXISTS events.clicks (
    id UUID DEFAULT gen_random_uuid(),
    impression_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID NOT NULL,
    device_id VARCHAR(255),
    session_id VARCHAR(255),
    country VARCHAR(2),
    platform VARCHAR(50),
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_clicks_impression ON events.clicks(impression_id);
CREATE INDEX IF NOT EXISTS idx_clicks_tenant ON events.clicks(tenant_id);
CREATE INDEX IF NOT EXISTS idx_clicks_app ON events.clicks(app_id);

-- Revenue events
CREATE TABLE IF NOT EXISTS events.revenue (
    id UUID DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID NOT NULL,
    impression_id UUID,
    event_type VARCHAR(50) CHECK (event_type IN ('impression', 'click', 'conversion', 'reward')),
    gross_revenue DECIMAL(12,6) NOT NULL,
    net_revenue DECIMAL(12,6),
    currency VARCHAR(3) DEFAULT 'USD',
    country VARCHAR(2),
    platform VARCHAR(50),
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_revenue_tenant ON events.revenue(tenant_id);
CREATE INDEX IF NOT EXISTS idx_revenue_app ON events.revenue(app_id);
CREATE INDEX IF NOT EXISTS idx_revenue_adapter ON events.revenue(adapter_id);
CREATE INDEX IF NOT EXISTS idx_revenue_country ON events.revenue(country);

-- SDK telemetry
CREATE TABLE IF NOT EXISTS events.sdk_telemetry (
    id UUID DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    device_id VARCHAR(255),
    session_id VARCHAR(255),
    event_type VARCHAR(100) NOT NULL,
    sdk_version VARCHAR(50),
    platform VARCHAR(50),
    os_version VARCHAR(50),
    device_model VARCHAR(100),
    payload JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_telemetry_tenant ON events.sdk_telemetry(tenant_id);
CREATE INDEX IF NOT EXISTS idx_telemetry_event ON events.sdk_telemetry(event_type);
CREATE INDEX IF NOT EXISTS idx_telemetry_sdk ON events.sdk_telemetry(sdk_version);

-- ============================================================================
-- PHASE 6: Agg Schema - Pre-computed Aggregates
-- ============================================================================

-- Hourly metrics (computed every hour)
CREATE TABLE IF NOT EXISTS agg.metrics_hourly (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID,
    placement_id UUID,
    adapter_id UUID,
    country VARCHAR(2),
    platform VARCHAR(50),
    hour TIMESTAMP WITH TIME ZONE NOT NULL,
    requests BIGINT DEFAULT 0,
    fills BIGINT DEFAULT 0,
    impressions BIGINT DEFAULT 0,
    clicks BIGINT DEFAULT 0,
    revenue DECIMAL(15,6) DEFAULT 0,
    fill_rate DECIMAL(5,4),
    ctr DECIMAL(5,4),
    ecpm DECIMAL(10,4),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tenant_id, app_id, placement_id, adapter_id, country, platform, hour)
);

CREATE INDEX IF NOT EXISTS idx_metrics_hourly_tenant ON agg.metrics_hourly(tenant_id);
CREATE INDEX IF NOT EXISTS idx_metrics_hourly_hour ON agg.metrics_hourly(hour);
CREATE INDEX IF NOT EXISTS idx_metrics_hourly_app ON agg.metrics_hourly(app_id);

-- Daily metrics (computed daily at midnight)
CREATE TABLE IF NOT EXISTS agg.metrics_daily (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID,
    placement_id UUID,
    adapter_id UUID,
    country VARCHAR(2),
    platform VARCHAR(50),
    date DATE NOT NULL,
    requests BIGINT DEFAULT 0,
    fills BIGINT DEFAULT 0,
    impressions BIGINT DEFAULT 0,
    clicks BIGINT DEFAULT 0,
    revenue DECIMAL(15,6) DEFAULT 0,
    unique_users BIGINT DEFAULT 0,
    sessions BIGINT DEFAULT 0,
    fill_rate DECIMAL(5,4),
    ctr DECIMAL(5,4),
    ecpm DECIMAL(10,4),
    arpdau DECIMAL(10,4),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tenant_id, app_id, placement_id, adapter_id, country, platform, date)
);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_tenant ON agg.metrics_daily(tenant_id);
CREATE INDEX IF NOT EXISTS idx_metrics_daily_date ON agg.metrics_daily(date);
CREATE INDEX IF NOT EXISTS idx_metrics_daily_app ON agg.metrics_daily(app_id);

-- Monthly rollups
CREATE TABLE IF NOT EXISTS agg.metrics_monthly (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID,
    adapter_id UUID,
    country VARCHAR(2),
    platform VARCHAR(50),
    month DATE NOT NULL,
    requests BIGINT DEFAULT 0,
    fills BIGINT DEFAULT 0,
    impressions BIGINT DEFAULT 0,
    clicks BIGINT DEFAULT 0,
    revenue DECIMAL(15,6) DEFAULT 0,
    unique_users BIGINT DEFAULT 0,
    fill_rate DECIMAL(5,4),
    ctr DECIMAL(5,4),
    ecpm DECIMAL(10,4),
    mom_growth DECIMAL(7,4),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tenant_id, app_id, adapter_id, country, platform, month)
);

CREATE INDEX IF NOT EXISTS idx_metrics_monthly_tenant ON agg.metrics_monthly(tenant_id);
CREATE INDEX IF NOT EXISTS idx_metrics_monthly_month ON agg.metrics_monthly(month);

-- Adapter performance rankings
CREATE TABLE IF NOT EXISTS agg.adapter_rankings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID,
    placement_id UUID,
    adapter_id UUID NOT NULL,
    period_type VARCHAR(20) CHECK (period_type IN ('daily', 'weekly', 'monthly')),
    period_start DATE NOT NULL,
    rank INTEGER,
    revenue DECIMAL(15,6),
    impressions BIGINT,
    fill_rate DECIMAL(5,4),
    ecpm DECIMAL(10,4),
    avg_latency_ms INTEGER,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tenant_id, app_id, placement_id, adapter_id, period_type, period_start)
);

CREATE INDEX IF NOT EXISTS idx_adapter_rankings_tenant ON agg.adapter_rankings(tenant_id);
CREATE INDEX IF NOT EXISTS idx_adapter_rankings_period ON agg.adapter_rankings(period_start);

-- Cohort analysis
CREATE TABLE IF NOT EXISTS agg.cohort_metrics (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    cohort_date DATE NOT NULL,
    day_number INTEGER NOT NULL CHECK (day_number >= 0),
    users BIGINT DEFAULT 0,
    retained_users BIGINT DEFAULT 0,
    sessions BIGINT DEFAULT 0,
    revenue DECIMAL(15,6) DEFAULT 0,
    retention_rate DECIMAL(5,4),
    arpu DECIMAL(10,4),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tenant_id, app_id, cohort_date, day_number)
);

CREATE INDEX IF NOT EXISTS idx_cohort_tenant ON agg.cohort_metrics(tenant_id);
CREATE INDEX IF NOT EXISTS idx_cohort_app ON agg.cohort_metrics(app_id);
CREATE INDEX IF NOT EXISTS idx_cohort_date ON agg.cohort_metrics(cohort_date);

-- ============================================================================
-- PHASE 7: Partition Management Functions
-- ============================================================================

-- Function to create daily partitions for events tables
CREATE OR REPLACE FUNCTION ops.create_daily_partition(
    p_table_name TEXT,
    p_schema_name TEXT,
    p_date DATE
) RETURNS VOID AS $$
DECLARE
    v_partition_name TEXT;
    v_start_date DATE;
    v_end_date DATE;
BEGIN
    v_partition_name := p_table_name || '_' || to_char(p_date, 'YYYYMMDD');
    v_start_date := p_date;
    v_end_date := p_date + INTERVAL '1 day';
    
    EXECUTE format(
        'CREATE TABLE IF NOT EXISTS %I.%I PARTITION OF %I.%I
         FOR VALUES FROM (%L) TO (%L)',
        p_schema_name, v_partition_name, p_schema_name, p_table_name,
        v_start_date, v_end_date
    );
    
    RAISE NOTICE 'Created partition: %.%', p_schema_name, v_partition_name;
END;
$$ LANGUAGE plpgsql;

-- Function to create monthly partitions for audit tables
CREATE OR REPLACE FUNCTION ops.create_monthly_partition(
    p_table_name TEXT,
    p_schema_name TEXT,
    p_date DATE
) RETURNS VOID AS $$
DECLARE
    v_partition_name TEXT;
    v_start_date DATE;
    v_end_date DATE;
BEGIN
    v_partition_name := p_table_name || '_' || to_char(p_date, 'YYYYMM');
    v_start_date := date_trunc('month', p_date);
    v_end_date := date_trunc('month', p_date) + INTERVAL '1 month';
    
    EXECUTE format(
        'CREATE TABLE IF NOT EXISTS %I.%I PARTITION OF %I.%I
         FOR VALUES FROM (%L) TO (%L)',
        p_schema_name, v_partition_name, p_schema_name, p_table_name,
        v_start_date, v_end_date
    );
    
    RAISE NOTICE 'Created partition: %.%', p_schema_name, v_partition_name;
END;
$$ LANGUAGE plpgsql;

-- Function to drop old partitions (retention policy)
CREATE OR REPLACE FUNCTION ops.drop_old_partitions(
    p_table_name TEXT,
    p_schema_name TEXT,
    p_retention_days INTEGER
) RETURNS INTEGER AS $$
DECLARE
    v_partition RECORD;
    v_count INTEGER := 0;
    v_cutoff_date DATE;
BEGIN
    v_cutoff_date := CURRENT_DATE - p_retention_days;
    
    FOR v_partition IN
        SELECT schemaname, tablename
        FROM pg_tables
        WHERE schemaname = p_schema_name
        AND tablename LIKE p_table_name || '_%'
        AND tablename ~ (p_table_name || '_[0-9]{8}$')
    LOOP
        -- Extract date from partition name and check if older than cutoff
        IF substring(v_partition.tablename from length(p_table_name) + 2)::DATE < v_cutoff_date THEN
            EXECUTE format('DROP TABLE IF EXISTS %I.%I', p_schema_name, v_partition.tablename);
            v_count := v_count + 1;
            RAISE NOTICE 'Dropped partition: %.%', p_schema_name, v_partition.tablename;
        END IF;
    END LOOP;
    
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- Autopilot: Create partitions for next 7 days
CREATE OR REPLACE FUNCTION ops.ensure_future_partitions() RETURNS VOID AS $$
DECLARE
    v_date DATE;
    v_tables TEXT[] := ARRAY['ad_requests', 'bid_responses', 'impressions', 'clicks', 'revenue', 'sdk_telemetry'];
    v_table TEXT;
BEGIN
    FOR i IN 0..6 LOOP
        v_date := CURRENT_DATE + i;
        FOREACH v_table IN ARRAY v_tables LOOP
            PERFORM ops.create_daily_partition(v_table, 'events', v_date);
        END LOOP;
    END LOOP;
    
    -- Also ensure next month partition for audit tables
    PERFORM ops.create_monthly_partition('audit_logs', 'ops', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
    PERFORM ops.create_monthly_partition('billing_audit', 'ops', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
    PERFORM ops.create_monthly_partition('job_logs', 'ops', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
    PERFORM ops.create_monthly_partition('vra_ledger', 'core', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
END;
$$ LANGUAGE plpgsql;

-- Autopilot: Apply retention policy
CREATE OR REPLACE FUNCTION ops.apply_retention_policy() RETURNS JSONB AS $$
DECLARE
    v_results JSONB := '{}';
    v_count INTEGER;
BEGIN
    -- Events: 90 days retention
    SELECT ops.drop_old_partitions('ad_requests', 'events', 90) INTO v_count;
    v_results := jsonb_set(v_results, '{ad_requests}', to_jsonb(v_count));
    
    SELECT ops.drop_old_partitions('bid_responses', 'events', 90) INTO v_count;
    v_results := jsonb_set(v_results, '{bid_responses}', to_jsonb(v_count));
    
    SELECT ops.drop_old_partitions('impressions', 'events', 90) INTO v_count;
    v_results := jsonb_set(v_results, '{impressions}', to_jsonb(v_count));
    
    SELECT ops.drop_old_partitions('clicks', 'events', 90) INTO v_count;
    v_results := jsonb_set(v_results, '{clicks}', to_jsonb(v_count));
    
    SELECT ops.drop_old_partitions('revenue', 'events', 90) INTO v_count;
    v_results := jsonb_set(v_results, '{revenue}', to_jsonb(v_count));
    
    SELECT ops.drop_old_partitions('sdk_telemetry', 'events', 30) INTO v_count;
    v_results := jsonb_set(v_results, '{sdk_telemetry}', to_jsonb(v_count));
    
    -- Hourly metrics: 30 days
    DELETE FROM agg.metrics_hourly WHERE hour < CURRENT_TIMESTAMP - INTERVAL '30 days';
    GET DIAGNOSTICS v_count = ROW_COUNT;
    v_results := jsonb_set(v_results, '{metrics_hourly}', to_jsonb(v_count));
    
    RETURN v_results;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- PHASE 8: Aggregation Functions
-- ============================================================================

-- Aggregate hourly metrics from events
CREATE OR REPLACE FUNCTION agg.compute_hourly_metrics(p_hour TIMESTAMP WITH TIME ZONE) RETURNS INTEGER AS $$
DECLARE
    v_count INTEGER;
BEGIN
    INSERT INTO agg.metrics_hourly (
        tenant_id, app_id, placement_id, adapter_id, country, platform, hour,
        requests, fills, impressions, clicks, revenue, fill_rate, ctr, ecpm
    )
    SELECT
        i.tenant_id,
        i.app_id,
        i.placement_id,
        i.adapter_id,
        i.country,
        i.platform,
        date_trunc('hour', i.created_at) as hour,
        0 as requests, -- Would need to join with ad_requests
        0 as fills,
        COUNT(*) as impressions,
        COALESCE(c.click_count, 0) as clicks,
        SUM(i.revenue) as revenue,
        0 as fill_rate,
        CASE WHEN COUNT(*) > 0 THEN COALESCE(c.click_count, 0)::DECIMAL / COUNT(*) ELSE 0 END as ctr,
        CASE WHEN COUNT(*) > 0 THEN SUM(i.revenue) * 1000 / COUNT(*) ELSE 0 END as ecpm
    FROM events.impressions i
    LEFT JOIN (
        SELECT impression_id, COUNT(*) as click_count
        FROM events.clicks
        WHERE created_at >= p_hour AND created_at < p_hour + INTERVAL '1 hour'
        GROUP BY impression_id
    ) c ON c.impression_id = i.id
    WHERE i.created_at >= p_hour AND i.created_at < p_hour + INTERVAL '1 hour'
    GROUP BY i.tenant_id, i.app_id, i.placement_id, i.adapter_id, i.country, i.platform, date_trunc('hour', i.created_at)
    ON CONFLICT (tenant_id, app_id, placement_id, adapter_id, country, platform, hour)
    DO UPDATE SET
        impressions = EXCLUDED.impressions,
        clicks = EXCLUDED.clicks,
        revenue = EXCLUDED.revenue,
        ctr = EXCLUDED.ctr,
        ecpm = EXCLUDED.ecpm,
        updated_at = NOW();
    
    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- Aggregate daily metrics from hourly
CREATE OR REPLACE FUNCTION agg.compute_daily_metrics(p_date DATE) RETURNS INTEGER AS $$
DECLARE
    v_count INTEGER;
BEGIN
    INSERT INTO agg.metrics_daily (
        tenant_id, app_id, placement_id, adapter_id, country, platform, date,
        requests, fills, impressions, clicks, revenue, fill_rate, ctr, ecpm
    )
    SELECT
        tenant_id,
        app_id,
        placement_id,
        adapter_id,
        country,
        platform,
        p_date as date,
        SUM(requests) as requests,
        SUM(fills) as fills,
        SUM(impressions) as impressions,
        SUM(clicks) as clicks,
        SUM(revenue) as revenue,
        CASE WHEN SUM(requests) > 0 THEN SUM(fills)::DECIMAL / SUM(requests) ELSE 0 END as fill_rate,
        CASE WHEN SUM(impressions) > 0 THEN SUM(clicks)::DECIMAL / SUM(impressions) ELSE 0 END as ctr,
        CASE WHEN SUM(impressions) > 0 THEN SUM(revenue) * 1000 / SUM(impressions) ELSE 0 END as ecpm
    FROM agg.metrics_hourly
    WHERE hour >= p_date AND hour < p_date + INTERVAL '1 day'
    GROUP BY tenant_id, app_id, placement_id, adapter_id, country, platform
    ON CONFLICT (tenant_id, app_id, placement_id, adapter_id, country, platform, date)
    DO UPDATE SET
        requests = EXCLUDED.requests,
        fills = EXCLUDED.fills,
        impressions = EXCLUDED.impressions,
        clicks = EXCLUDED.clicks,
        revenue = EXCLUDED.revenue,
        fill_rate = EXCLUDED.fill_rate,
        ctr = EXCLUDED.ctr,
        ecpm = EXCLUDED.ecpm,
        updated_at = NOW();
    
    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- PHASE 9: Autopilot Health Check Function
-- ============================================================================

CREATE OR REPLACE FUNCTION ops.run_health_check() RETURNS JSONB AS $$
DECLARE
    v_checks JSONB := '{}';
    v_state VARCHAR(20) := 'OK';
    v_temp RECORD;
    v_disk_pct DECIMAL;
    v_conn_pct DECIMAL;
    v_repl_lag INTERVAL;
BEGIN
    -- Check 1: Connection usage
    SELECT 
        (SELECT count(*) FROM pg_stat_activity)::DECIMAL / 
        current_setting('max_connections')::DECIMAL * 100
    INTO v_conn_pct;
    
    v_checks := jsonb_set(v_checks, '{connections}', jsonb_build_object(
        'usage_pct', v_conn_pct,
        'status', CASE WHEN v_conn_pct > 80 THEN 'WARN' WHEN v_conn_pct > 95 THEN 'FAIL' ELSE 'OK' END
    ));
    
    IF v_conn_pct > 95 THEN v_state := 'FAIL';
    ELSIF v_conn_pct > 80 AND v_state != 'FAIL' THEN v_state := 'DEGRADED';
    END IF;
    
    -- Check 2: Long-running queries
    SELECT COUNT(*) INTO v_temp
    FROM pg_stat_activity
    WHERE state = 'active'
    AND query_start < NOW() - INTERVAL '5 minutes'
    AND query NOT LIKE 'autovacuum%';
    
    v_checks := jsonb_set(v_checks, '{long_queries}', jsonb_build_object(
        'count', v_temp.count,
        'status', CASE WHEN v_temp.count > 5 THEN 'WARN' WHEN v_temp.count > 10 THEN 'FAIL' ELSE 'OK' END
    ));
    
    IF v_temp.count > 10 THEN v_state := 'FAIL';
    ELSIF v_temp.count > 5 AND v_state != 'FAIL' THEN v_state := 'DEGRADED';
    END IF;
    
    -- Check 3: Dead tuples (bloat)
    SELECT COUNT(*) INTO v_temp
    FROM pg_stat_user_tables
    WHERE n_dead_tup > n_live_tup * 0.2
    AND n_live_tup > 1000;
    
    v_checks := jsonb_set(v_checks, '{bloat}', jsonb_build_object(
        'tables_with_bloat', v_temp.count,
        'status', CASE WHEN v_temp.count > 5 THEN 'WARN' ELSE 'OK' END
    ));
    
    -- Check 4: Cache hit ratio
    SELECT 
        CASE WHEN sum(blks_hit) + sum(blks_read) > 0 
        THEN sum(blks_hit)::DECIMAL / (sum(blks_hit) + sum(blks_read)) * 100 
        ELSE 100 END
    INTO v_temp
    FROM pg_stat_database
    WHERE datname = current_database();
    
    v_checks := jsonb_set(v_checks, '{cache_hit_ratio}', jsonb_build_object(
        'ratio', v_temp,
        'status', CASE WHEN v_temp < 90 THEN 'WARN' WHEN v_temp < 80 THEN 'FAIL' ELSE 'OK' END
    ));
    
    IF v_temp < 80 THEN v_state := 'FAIL';
    ELSIF v_temp < 90 AND v_state != 'FAIL' THEN v_state := 'DEGRADED';
    END IF;
    
    -- Check 5: Partition health
    SELECT COUNT(*) INTO v_temp
    FROM information_schema.tables t
    LEFT JOIN pg_partitioned_table pt ON t.table_name = pt.relid::regclass::text
    WHERE t.table_schema IN ('events', 'ops', 'core')
    AND pt.relid IS NOT NULL;
    
    v_checks := jsonb_set(v_checks, '{partitions}', jsonb_build_object(
        'partitioned_tables', v_temp.count,
        'status', 'OK'
    ));
    
    -- Update status table
    UPDATE ops.autopilot_status SET
        state = v_state,
        last_check = NOW(),
        checks = v_checks,
        updated_at = NOW()
    WHERE id = 1;
    
    RETURN jsonb_build_object(
        'state', v_state,
        'checks', v_checks,
        'timestamp', NOW()
    );
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- PHASE 10: Initial Partitions for Current Period
-- ============================================================================

-- Create partitions for current week
DO $$
DECLARE
    v_date DATE;
BEGIN
    FOR i IN -3..7 LOOP
        v_date := CURRENT_DATE + i;
        
        -- Events tables
        PERFORM ops.create_daily_partition('ad_requests', 'events', v_date);
        PERFORM ops.create_daily_partition('bid_responses', 'events', v_date);
        PERFORM ops.create_daily_partition('impressions', 'events', v_date);
        PERFORM ops.create_daily_partition('clicks', 'events', v_date);
        PERFORM ops.create_daily_partition('revenue', 'events', v_date);
        PERFORM ops.create_daily_partition('sdk_telemetry', 'events', v_date);
    END LOOP;
    
    -- Current and next month for audit tables
    PERFORM ops.create_monthly_partition('audit_logs', 'ops', date_trunc('month', CURRENT_DATE)::DATE);
    PERFORM ops.create_monthly_partition('audit_logs', 'ops', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
    
    PERFORM ops.create_monthly_partition('billing_audit', 'ops', date_trunc('month', CURRENT_DATE)::DATE);
    PERFORM ops.create_monthly_partition('billing_audit', 'ops', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
    
    PERFORM ops.create_monthly_partition('job_logs', 'ops', date_trunc('month', CURRENT_DATE)::DATE);
    PERFORM ops.create_monthly_partition('job_logs', 'ops', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
    
    PERFORM ops.create_monthly_partition('vra_ledger', 'core', date_trunc('month', CURRENT_DATE)::DATE);
    PERFORM ops.create_monthly_partition('vra_ledger', 'core', date_trunc('month', CURRENT_DATE + INTERVAL '1 month')::DATE);
END $$;

-- ============================================================================
-- PHASE 11: Comments for Documentation
-- ============================================================================

COMMENT ON SCHEMA core IS 'Business entities: publishers, apps, placements, adapters, users';
COMMENT ON SCHEMA config IS 'Runtime configuration: feature flags, AB tests, migrations';
COMMENT ON SCHEMA ops IS 'Operational data: autopilot, audit logs, job logs';
COMMENT ON SCHEMA events IS 'High-volume event data: partitioned by day, 90-day retention';
COMMENT ON SCHEMA agg IS 'Pre-computed aggregates: hourly, daily, monthly metrics';

COMMENT ON TABLE ops.autopilot_status IS 'Singleton table tracking database health state (OK/DEGRADED/FAIL)';
COMMENT ON TABLE ops.autopilot_jobs IS 'Log of scheduled maintenance tasks run by autopilot';

COMMENT ON FUNCTION ops.run_health_check() IS 'Run all health checks and update autopilot_status';
COMMENT ON FUNCTION ops.ensure_future_partitions() IS 'Create partitions for next 7 days of events';
COMMENT ON FUNCTION ops.apply_retention_policy() IS 'Drop old partitions per retention policy';

-- Grant execute on functions
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA ops TO app_service;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA agg TO app_service;
