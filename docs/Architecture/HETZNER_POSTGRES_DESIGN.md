# Hetzner Postgres-Only Infrastructure Design

**Status:** Deployed (Production)  
**Date:** 2025-12-13  
**Updated:** 2025-01-06 (IP corrections, monitoring stack addition)
**Target:** Long-term heavy load, self-managing, speed-first architecture

---

## Related Documentation

| Document | Purpose |
|----------|---------|
| [Monitoring Infrastructure](../Monitoring/MONITORING_INFRASTRUCTURE.md) | Prometheus, Grafana, Alertmanager setup |
| [Production Deployment](../Internal/Infrastructure/production-deployment.md) | Deployment procedures |

---

## 1. Infrastructure Overview

### Server Topology

```
┌─────────────────────────────────────────────────────────────────┐
│                        INTERNET                                  │
└─────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    APP SERVER (46.62.252.96)                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   Backend   │  │   Console   │  │   Monitoring Stack      │  │
│  │  (Node.js)  │  │  (Next.js)  │  │  Prometheus/Grafana     │  │
│  │   :8080     │  │    :3000    │  │  :9090/:3001/:9093      │  │
│  └──────┬──────┘  └─────────────┘  └───────────┬─────────────┘  │
│         │                                       │                │
│  ┌──────┴──────┐                     ┌─────────┴─────────┐      │
│  │    Redis    │                     │  Node Exporter    │      │
│  │   (Cache)   │                     │  cAdvisor         │      │
│  └─────────────┘                     └───────────────────┘      │
│         │                                       │                │
│         └───────────────────┬───────────────────┘                │
└─────────────────────────────┼───────────────────────────────────┘
                              │
                    PRIVATE NETWORK ONLY
                    (Hetzner vSwitch/VLAN)
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    DB SERVER (77.42.44.35)                       │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                    PostgreSQL 16  :5432                     ││
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌────────┐││
│  │  │  core   │ │  config │ │   ops   │ │  events │ │  agg   │││
│  │  │ schema  │ │ schema  │ │ schema  │ │ schema  │ │ schema │││
│  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └────────┘││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │           Postgres Exporter :9187 (for Prometheus)          ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

### Network Security Model

| From | To | Protocol | Allowed |
|------|-----|----------|---------|
| Internet | App Server:80/443 | HTTPS | ✅ |
| Internet | App Server:22 | SSH | ✅ (key-only) |
| Internet | App Server:8080 | Backend API | ✅ |
| Internet | App Server:3000 | Console | ✅ |
| Internet | App Server:3001 | Grafana | ✅ |
| Internet | App Server:9090 | Prometheus | ✅ |
| Internet | DB Server:* | Any | ⚠️ (currently accessible, recommend firewall) |
| App Server | DB Server:5432 | PostgreSQL | ✅ |
| App Server | DB Server:9187 | Postgres Exporter | ✅ |

### SSH Access

```bash
# App Server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# DB Server
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35

# Database connection
PGPASSWORD='ApexProd2025!' psql -h 77.42.44.35 -U postgres -d adproject
```

---

## 2. Database Schema Architecture

### Schema Separation Strategy

Separate workload domains into PostgreSQL schemas to isolate concerns:

```sql
-- Schema creation
CREATE SCHEMA IF NOT EXISTS core;    -- Tenants, apps, users, RBAC, API keys
CREATE SCHEMA IF NOT EXISTS config;  -- Mediation configs, versioning, experiments
CREATE SCHEMA IF NOT EXISTS ops;     -- Migration Studio, validations, health
CREATE SCHEMA IF NOT EXISTS events;  -- Raw event facts (partitioned)
CREATE SCHEMA IF NOT EXISTS agg;     -- Rollups (minute/hour/day)
```

### 2.1 Core Schema (OLTP - Low Volume, High Consistency)

```sql
-- core.publishers
CREATE TABLE core.publishers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_name TEXT NOT NULL,
    tier TEXT NOT NULL DEFAULT 'free' CHECK (tier IN ('free','starter','growth','scale','enterprise')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- core.users
CREATE TABLE core.users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'customer' CHECK (role IN ('admin','publisher','readonly','customer')),
    mfa_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_users_publisher ON core.users(publisher_id);
CREATE INDEX idx_users_email ON core.users(email);

-- core.apps
CREATE TABLE core.apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    bundle_id TEXT NOT NULL,
    platform TEXT NOT NULL CHECK (platform IN ('ios','android','web')),
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(publisher_id, bundle_id, platform)
);
CREATE INDEX idx_apps_publisher ON core.apps(publisher_id);

-- core.placements
CREATE TABLE core.placements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES core.apps(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    ad_format TEXT NOT NULL CHECK (ad_format IN ('banner','interstitial','rewarded','native')),
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_placements_app ON core.placements(app_id);

-- core.api_keys (SDK authentication)
CREATE TABLE core.api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    app_id UUID REFERENCES core.apps(id) ON DELETE CASCADE,
    key_hash TEXT NOT NULL, -- bcrypt hash of key
    key_prefix TEXT NOT NULL, -- First 8 chars for identification
    name TEXT,
    scopes TEXT[] NOT NULL DEFAULT ARRAY['rtb'],
    rate_limit_rpm INTEGER NOT NULL DEFAULT 1000,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ
);
CREATE INDEX idx_api_keys_publisher ON core.api_keys(publisher_id);
CREATE INDEX idx_api_keys_prefix ON core.api_keys(key_prefix);
CREATE UNIQUE INDEX idx_api_keys_hash ON core.api_keys(key_hash) WHERE is_active = TRUE;

-- core.adapters (ad network definitions)
CREATE TABLE core.adapters (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    adapter_type TEXT NOT NULL DEFAULT 'rtb',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config_schema JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### 2.2 Config Schema (Versioned Configuration)

```sql
-- config.adapter_configs (publisher-specific adapter settings)
CREATE TABLE config.adapter_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    adapter_id UUID NOT NULL REFERENCES core.adapters(id),
    config JSONB NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(publisher_id, adapter_id)
);
CREATE INDEX idx_adapter_configs_publisher ON config.adapter_configs(publisher_id);

-- config.waterfall_rules (mediation logic)
CREATE TABLE config.waterfall_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    placement_id UUID NOT NULL REFERENCES core.placements(id) ON DELETE CASCADE,
    adapter_id UUID NOT NULL REFERENCES core.adapters(id),
    priority INTEGER NOT NULL,
    floor_cpm_micros BIGINT NOT NULL DEFAULT 0,
    timeout_ms INTEGER NOT NULL DEFAULT 5000,
    geo_targets TEXT[], -- ISO country codes
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(placement_id, adapter_id)
);
CREATE INDEX idx_waterfall_rules_placement ON config.waterfall_rules(placement_id);

-- config.experiments (A/B testing, VRA)
CREATE TABLE config.experiments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    experiment_type TEXT NOT NULL CHECK (experiment_type IN ('ab_test','migration','vra')),
    config JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    traffic_percent INTEGER NOT NULL DEFAULT 0 CHECK (traffic_percent >= 0 AND traffic_percent <= 100),
    version INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    activated_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);
CREATE INDEX idx_experiments_publisher ON config.experiments(publisher_id);
CREATE INDEX idx_experiments_status ON config.experiments(status);

-- config.config_versions (audit trail for config changes)
CREATE TABLE config.config_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type TEXT NOT NULL, -- 'adapter_config', 'waterfall_rule', 'experiment'
    entity_id UUID NOT NULL,
    version INTEGER NOT NULL,
    config_snapshot JSONB NOT NULL,
    changed_by UUID REFERENCES core.users(id),
    change_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_config_versions_entity ON config.config_versions(entity_type, entity_id);
```

### 2.3 Ops Schema (Migration Studio, Health, Incidents)

```sql
-- ops.migration_jobs (Migration Studio imports)
CREATE TABLE ops.migration_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES core.publishers(id) ON DELETE CASCADE,
    experiment_id UUID REFERENCES config.experiments(id),
    source_mediation TEXT NOT NULL, -- 'ironsource', 'max', 'admob'
    status TEXT NOT NULL DEFAULT 'pending',
    config_data JSONB NOT NULL DEFAULT '{}',
    progress_percent INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_migration_jobs_publisher ON ops.migration_jobs(publisher_id);
CREATE INDEX idx_migration_jobs_status ON ops.migration_jobs(status);

-- ops.validation_runs (import/migration validations)
CREATE TABLE ops.validation_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NOT NULL REFERENCES ops.migration_jobs(id) ON DELETE CASCADE,
    validation_type TEXT NOT NULL,
    passed BOOLEAN NOT NULL,
    results JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_validation_runs_job ON ops.validation_runs(job_id);

-- ops.adapter_health (real-time adapter status)
CREATE TABLE ops.adapter_health (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    adapter_id UUID NOT NULL REFERENCES core.adapters(id),
    health_status TEXT NOT NULL DEFAULT 'healthy',
    latency_p50_ms INTEGER,
    latency_p95_ms INTEGER,
    error_rate_percent NUMERIC(5,2),
    last_success_at TIMESTAMPTZ,
    last_failure_at TIMESTAMPTZ,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_adapter_health_adapter ON ops.adapter_health(adapter_id);
CREATE INDEX idx_adapter_health_status ON ops.adapter_health(health_status);

-- ops.incidents (system incidents)
CREATE TABLE ops.incidents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    severity TEXT NOT NULL CHECK (severity IN ('info','warning','critical')),
    category TEXT NOT NULL, -- 'database', 'adapter', 'billing', 'security'
    title TEXT NOT NULL,
    description TEXT,
    metadata JSONB NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'open',
    acknowledged_by UUID REFERENCES core.users(id),
    acknowledged_at TIMESTAMPTZ,
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_incidents_status ON ops.incidents(status);
CREATE INDEX idx_incidents_created ON ops.incidents(created_at DESC);

-- ops.autopilot_status (DB Autopilot state)
CREATE TABLE ops.autopilot_status (
    id SERIAL PRIMARY KEY,
    health TEXT NOT NULL CHECK (health IN ('OK','DEGRADED','FAIL')),
    reasons TEXT[] NOT NULL DEFAULT '{}',
    disk_percent NUMERIC(5,2),
    disk_days_to_85 INTEGER,
    swap_in_use BOOLEAN NOT NULL DEFAULT FALSE,
    connection_saturation_percent NUMERIC(5,2),
    vacuum_lag_hours NUMERIC(10,2),
    wal_bytes_per_min BIGINT,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_autopilot_recorded ON ops.autopilot_status(recorded_at DESC);

-- ops.autopilot_jobs (scheduled task ledger)
CREATE TABLE ops.autopilot_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_name TEXT NOT NULL,
    job_type TEXT NOT NULL, -- 'partition_create', 'partition_drop', 'vacuum', 'analyze'
    status TEXT NOT NULL DEFAULT 'scheduled',
    scheduled_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    duration_ms INTEGER,
    exit_code INTEGER,
    output TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_autopilot_jobs_status ON ops.autopilot_jobs(status);
CREATE INDEX idx_autopilot_jobs_scheduled ON ops.autopilot_jobs(scheduled_at);
```

### 2.4 Events Schema (Partitioned - High Volume)

```sql
-- events.impressions (PARTITIONED BY DAY, SUB-PARTITIONED BY tenant hash)
CREATE TABLE events.impressions (
    id BIGSERIAL,
    tenant_id UUID NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    request_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID,
    ad_format TEXT NOT NULL,
    country_code CHAR(2) NOT NULL DEFAULT 'ZZ',
    device_type TEXT,
    os TEXT,
    filled BOOLEAN NOT NULL DEFAULT FALSE,
    revenue_micros BIGINT NOT NULL DEFAULT 0,
    latency_ms INTEGER,
    error_code INTEGER,
    is_test BOOLEAN NOT NULL DEFAULT FALSE,
    extras JSONB, -- Unindexed catch-all
    PRIMARY KEY (event_time, id, tenant_id)
) PARTITION BY RANGE (event_time);

-- Create monthly partitions (daily would be better for very high volume)
-- Partitions will be auto-created by Autopilot

-- Idempotency constraint
CREATE UNIQUE INDEX idx_impressions_idempotent 
    ON events.impressions (tenant_id, request_id, event_time);

-- Common query indexes (per-partition, minimal)
CREATE INDEX idx_impressions_tenant_time 
    ON events.impressions (tenant_id, event_time DESC);

-- BRIN for large time scans (tiny index, huge benefit)
CREATE INDEX idx_impressions_time_brin 
    ON events.impressions USING BRIN (event_time) WITH (pages_per_range = 128);

-- events.clicks
CREATE TABLE events.clicks (
    id BIGSERIAL,
    tenant_id UUID NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    impression_id BIGINT,
    request_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID,
    click_url TEXT,
    time_to_click_ms INTEGER,
    is_verified BOOLEAN NOT NULL DEFAULT FALSE,
    extras JSONB,
    PRIMARY KEY (event_time, id, tenant_id)
) PARTITION BY RANGE (event_time);

CREATE UNIQUE INDEX idx_clicks_idempotent 
    ON events.clicks (tenant_id, request_id, event_time);
CREATE INDEX idx_clicks_tenant_time 
    ON events.clicks (tenant_id, event_time DESC);

-- events.revenue
CREATE TABLE events.revenue (
    id BIGSERIAL,
    tenant_id UUID NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    impression_id BIGINT,
    request_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID,
    revenue_type TEXT NOT NULL DEFAULT 'impression',
    revenue_micros BIGINT NOT NULL,
    currency CHAR(3) NOT NULL DEFAULT 'USD',
    exchange_rate NUMERIC(12,6),
    reconciliation_status TEXT NOT NULL DEFAULT 'pending',
    extras JSONB,
    PRIMARY KEY (event_time, id, tenant_id)
) PARTITION BY RANGE (event_time);

CREATE UNIQUE INDEX idx_revenue_idempotent 
    ON events.revenue (tenant_id, request_id, event_time);
CREATE INDEX idx_revenue_tenant_time 
    ON events.revenue (tenant_id, event_time DESC);
CREATE INDEX idx_revenue_recon_status 
    ON events.revenue (reconciliation_status) WHERE reconciliation_status != 'matched';

-- events.errors
CREATE TABLE events.errors (
    id BIGSERIAL,
    tenant_id UUID NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    request_id UUID,
    app_id UUID NOT NULL,
    placement_id UUID,
    adapter_id UUID,
    error_code INTEGER NOT NULL,
    error_message TEXT,
    stage TEXT NOT NULL, -- 'bid', 'fill', 'render', 'click'
    extras JSONB,
    PRIMARY KEY (event_time, id, tenant_id)
) PARTITION BY RANGE (event_time);

CREATE INDEX idx_errors_tenant_time 
    ON events.errors (tenant_id, event_time DESC);
CREATE INDEX idx_errors_code 
    ON events.errors (error_code, event_time DESC);
```

### 2.5 Agg Schema (Rollups - Analytics Queries)

```sql
-- agg.impressions_hourly
CREATE TABLE agg.impressions_hourly (
    bucket_hour TIMESTAMPTZ NOT NULL,
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID NOT NULL,
    country_code CHAR(2) NOT NULL DEFAULT 'ZZ',
    ad_format TEXT NOT NULL,
    -- Metrics
    impression_count BIGINT NOT NULL DEFAULT 0,
    filled_count BIGINT NOT NULL DEFAULT 0,
    click_count BIGINT NOT NULL DEFAULT 0,
    revenue_micros BIGINT NOT NULL DEFAULT 0,
    error_count BIGINT NOT NULL DEFAULT 0,
    timeout_count BIGINT NOT NULL DEFAULT 0,
    -- Latency approximations (reservoir sampling or t-digest)
    latency_sum_ms BIGINT NOT NULL DEFAULT 0,
    latency_count BIGINT NOT NULL DEFAULT 0,
    latency_p50_approx INTEGER,
    latency_p95_approx INTEGER,
    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (bucket_hour, tenant_id, app_id, placement_id, adapter_id, country_code, ad_format)
) PARTITION BY RANGE (bucket_hour);

-- agg.impressions_daily (for dashboards)
CREATE TABLE agg.impressions_daily (
    bucket_date DATE NOT NULL,
    tenant_id UUID NOT NULL,
    app_id UUID NOT NULL,
    placement_id UUID NOT NULL,
    adapter_id UUID NOT NULL,
    country_code CHAR(2) NOT NULL DEFAULT 'ZZ',
    ad_format TEXT NOT NULL,
    -- Metrics
    impression_count BIGINT NOT NULL DEFAULT 0,
    filled_count BIGINT NOT NULL DEFAULT 0,
    click_count BIGINT NOT NULL DEFAULT 0,
    revenue_micros BIGINT NOT NULL DEFAULT 0,
    error_count BIGINT NOT NULL DEFAULT 0,
    timeout_count BIGINT NOT NULL DEFAULT 0,
    -- Computed metrics
    fill_rate NUMERIC(5,4) GENERATED ALWAYS AS (
        CASE WHEN impression_count > 0 THEN filled_count::NUMERIC / impression_count ELSE 0 END
    ) STORED,
    ctr NUMERIC(7,6) GENERATED ALWAYS AS (
        CASE WHEN impression_count > 0 THEN click_count::NUMERIC / impression_count ELSE 0 END
    ) STORED,
    ecpm_micros BIGINT GENERATED ALWAYS AS (
        CASE WHEN impression_count > 0 THEN (revenue_micros * 1000) / impression_count ELSE 0 END
    ) STORED,
    -- Latency
    latency_p50_approx INTEGER,
    latency_p95_approx INTEGER,
    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (bucket_date, tenant_id, app_id, placement_id, adapter_id, country_code, ad_format)
) PARTITION BY RANGE (bucket_date);

CREATE INDEX idx_daily_tenant_date ON agg.impressions_daily (tenant_id, bucket_date DESC);
CREATE INDEX idx_daily_adapter ON agg.impressions_daily (adapter_id, bucket_date DESC);

-- agg.experiment_results (VRA/A-B test outcomes)
CREATE TABLE agg.experiment_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id UUID NOT NULL REFERENCES config.experiments(id),
    bucket_date DATE NOT NULL,
    variant TEXT NOT NULL, -- 'control', 'treatment'
    -- Metrics
    impression_count BIGINT NOT NULL DEFAULT 0,
    revenue_micros BIGINT NOT NULL DEFAULT 0,
    fill_rate NUMERIC(5,4),
    ecpm_micros BIGINT,
    latency_p50_ms INTEGER,
    latency_p95_ms INTEGER,
    error_rate NUMERIC(5,4),
    -- Statistical significance
    confidence_level NUMERIC(5,4),
    p_value NUMERIC(10,8),
    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(experiment_id, bucket_date, variant)
);
CREATE INDEX idx_experiment_results_experiment ON agg.experiment_results(experiment_id);
```

---

## 3. Partition Management Strategy

### 3.1 Partition Creation (Autopilot Job)

```sql
-- Function to create future partitions
CREATE OR REPLACE FUNCTION ops.create_future_partitions(
    p_table_name TEXT,
    p_schema_name TEXT DEFAULT 'events',
    p_days_ahead INTEGER DEFAULT 7
) RETURNS INTEGER AS $$
DECLARE
    v_start_date DATE;
    v_end_date DATE;
    v_partition_name TEXT;
    v_created INTEGER := 0;
BEGIN
    FOR i IN 0..p_days_ahead LOOP
        v_start_date := CURRENT_DATE + i;
        v_end_date := v_start_date + 1;
        v_partition_name := p_table_name || '_p_' || TO_CHAR(v_start_date, 'YYYYMMDD');
        
        -- Check if partition exists
        IF NOT EXISTS (
            SELECT 1 FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.relname = v_partition_name
            AND n.nspname = p_schema_name
        ) THEN
            EXECUTE format(
                'CREATE TABLE %I.%I PARTITION OF %I.%I FOR VALUES FROM (%L) TO (%L)',
                p_schema_name, v_partition_name,
                p_schema_name, p_table_name,
                v_start_date, v_end_date
            );
            v_created := v_created + 1;
        END IF;
    END LOOP;
    
    RETURN v_created;
END;
$$ LANGUAGE plpgsql;
```

### 3.2 Partition Retention (Drop Old Partitions)

```sql
-- Function to drop old partitions (retention enforcement)
CREATE OR REPLACE FUNCTION ops.drop_old_partitions(
    p_table_name TEXT,
    p_schema_name TEXT DEFAULT 'events',
    p_retention_days INTEGER DEFAULT 90
) RETURNS INTEGER AS $$
DECLARE
    v_cutoff_date DATE;
    v_partition RECORD;
    v_dropped INTEGER := 0;
BEGIN
    v_cutoff_date := CURRENT_DATE - p_retention_days;
    
    FOR v_partition IN
        SELECT c.relname AS partition_name
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_inherits i ON i.inhrelid = c.oid
        JOIN pg_class parent ON parent.oid = i.inhparent
        WHERE n.nspname = p_schema_name
        AND parent.relname = p_table_name
        AND c.relname ~ '_p_[0-9]{8}$'
        AND SUBSTRING(c.relname FROM '_p_([0-9]{8})$')::DATE < v_cutoff_date
    LOOP
        EXECUTE format('DROP TABLE %I.%I', p_schema_name, v_partition.partition_name);
        v_dropped := v_dropped + 1;
    END LOOP;
    
    RETURN v_dropped;
END;
$$ LANGUAGE plpgsql;
```

### 3.3 Retention Policy Configuration

| Table | Retention | Partition Size |
|-------|-----------|----------------|
| `events.impressions` | 90 days | Daily |
| `events.clicks` | 90 days | Daily |
| `events.revenue` | 365 days | Daily |
| `events.errors` | 30 days | Daily |
| `agg.impressions_hourly` | 90 days | Weekly |
| `agg.impressions_daily` | 730 days (2 years) | Monthly |
| `agg.experiment_results` | Forever | None |

---

## 4. Runtime Configuration (postgresql.conf)

```ini
# Connection Management
max_connections = 100
reserved_connections = 3

# Memory
shared_buffers = 1GB                    # 25% of RAM for 4GB server
effective_cache_size = 3GB              # 75% of RAM
work_mem = 16MB                         # Per-operation memory
maintenance_work_mem = 256MB            # For vacuum, index creation

# WAL Configuration
wal_level = replica
max_wal_size = 2GB
min_wal_size = 512MB
checkpoint_completion_target = 0.9

# Query Timeouts (prevent runaway queries)
statement_timeout = '30s'
lock_timeout = '10s'
idle_in_transaction_session_timeout = '60s'

# Autovacuum (critical for partitioned tables)
autovacuum = on
autovacuum_max_workers = 4
autovacuum_naptime = '30s'
autovacuum_vacuum_threshold = 50
autovacuum_analyze_threshold = 50
autovacuum_vacuum_scale_factor = 0.05   # More aggressive for large tables
autovacuum_analyze_scale_factor = 0.02

# Logging
log_min_duration_statement = 1000       # Log queries > 1s
log_checkpoints = on
log_lock_waits = on
log_autovacuum_min_duration = 1000

# Extensions
shared_preload_libraries = 'pg_stat_statements'

# pg_stat_statements
pg_stat_statements.max = 10000
pg_stat_statements.track = 'all'
```

---

## 5. DB Autopilot Service

### 5.1 Health State Machine

```
        ┌──────────────────────────────────────────────┐
        │                                              │
        ▼                                              │
    ┌───────┐                                          │
    │  OK   │◄─────────────────────────────────────────┤
    └───┬───┘                                          │
        │                                              │
        │ disk >= 90% OR                               │
        │ swap_in_use OR                               │
        │ connections >= 90% OR                        │
        │ vacuum_lag > 24h OR                          │
        │ WAL anomaly                                  │
        ▼                                              │
   ┌──────────┐                                        │
   │ DEGRADED │────────────────────────────────────────┤
   └────┬─────┘         All conditions clear           │
        │                                              │
        │ disk >= 95% OR                               │
        │ DB restart loop OR                           │
        │ critical health check fail                   │
        ▼                                              │
    ┌───────┐                                          │
    │ FAIL  │──────────────────────────────────────────┘
    └───────┘         Manual intervention + clear
```

### 5.2 Autopilot Service Architecture

```python
# /opt/autopilot/autopilot.py (runs on DB server)

class DBAutopilot:
    """
    Self-managing database service that:
    1. Monitors health continuously
    2. Creates partitions ahead of time
    3. Drops old partitions per retention policy
    4. Alerts on degraded states
    5. Exposes status via local API
    """
    
    def __init__(self):
        self.health = 'OK'
        self.reasons = []
        self.forecasts = {}
        
    def check_health(self):
        """Run every 30 seconds"""
        checks = []
        
        # Disk usage
        disk = self.get_disk_usage()
        if disk >= 95:
            self.health = 'FAIL'
            checks.append(f'CRITICAL: Disk at {disk}%')
        elif disk >= 90:
            self.health = 'DEGRADED'
            checks.append(f'WARNING: Disk at {disk}%')
            
        # Swap usage
        if self.is_swap_in_use():
            self.health = 'DEGRADED'
            checks.append('WARNING: Swap in use')
            
        # Connection saturation
        conn_pct = self.get_connection_percent()
        if conn_pct >= 90:
            self.health = 'DEGRADED'
            checks.append(f'WARNING: Connections at {conn_pct}%')
            
        # Vacuum lag
        vacuum_lag = self.get_vacuum_lag_hours()
        if vacuum_lag > 24:
            self.health = 'DEGRADED'
            checks.append(f'WARNING: Vacuum lag {vacuum_lag}h')
            
        # Freeze risk
        if self.check_freeze_risk():
            self.health = 'DEGRADED'
            checks.append('WARNING: Approaching XID wraparound')
            
        self.reasons = checks
        return self.health
        
    def calculate_forecasts(self):
        """Predict capacity issues"""
        disk_growth_per_day = self.get_disk_growth_rate()
        current_disk = self.get_disk_usage()
        
        if disk_growth_per_day > 0:
            days_to_85 = (85 - current_disk) / disk_growth_per_day
            self.forecasts['days_to_85_percent'] = max(0, int(days_to_85))
        else:
            self.forecasts['days_to_85_percent'] = 999
            
    def run_scheduled_jobs(self):
        """Execute partition management"""
        # Create future partitions (7 days ahead)
        for table in ['impressions', 'clicks', 'revenue', 'errors']:
            self.create_partitions(f'events.{table}', days_ahead=7)
            
        # Drop old partitions per retention
        self.drop_old_partitions('events.impressions', retention_days=90)
        self.drop_old_partitions('events.clicks', retention_days=90)
        self.drop_old_partitions('events.revenue', retention_days=365)
        self.drop_old_partitions('events.errors', retention_days=30)
```

### 5.3 Autopilot Status Endpoint

```bash
# Local-only HTTP API on DB server (port 9100)
GET /health        → { "status": "OK|DEGRADED|FAIL", "reasons": [...] }
GET /metrics       → Prometheus metrics
GET /jobs          → Job ledger (last runs, durations, exit codes)
GET /partitions    → Partition inventory (sizes, dates, drop schedule)
GET /forecasts     → Capacity predictions
```

---

## 6. Admin Control Panel

### 6.1 Architecture

The Admin Control Panel runs on the App Server and connects to:
- **Backend API** (for business data)
- **Autopilot API** (for DB health, via private network to DB server)
- **Host Telemetry** (local app server metrics)

### 6.2 Control Panel Features

| Section | Data Source | Actions |
|---------|-------------|---------|
| **Overview** | Autopilot API | View health badge, disk forecast, incidents |
| **Performance** | pg_stat_statements | View top queries, lock waits, slow queries |
| **Capacity** | Autopilot API | View partition sizes, retention schedule |
| **Automation** | Autopilot API | View job freshness, trigger partition ops |
| **Security** | Backend API | View auth failures, config changes |
| **Business** | Backend API | Fill/win/error rates, revenue, experiments |

### 6.3 Safe Actions (Whitelisted)

All actions require:
1. Pre-check gates (verify safe to execute)
2. Audit logging (who/when/what/result)
3. Post-state verification

| Action | Risk Level | Pre-checks |
|--------|------------|------------|
| Enter/Exit DEGRADED | Low | Manual acknowledgment |
| Run deep healthcheck | Low | None |
| Rotate logs now | Low | Disk space check |
| Create future partitions | Low | None |
| Drop old partitions | Medium | Retention policy check, backup exists |
| Schedule maintenance window | Medium | No active incidents |
| Restart PostgreSQL | High | No active transactions, cooldown timer |

### 6.4 RBAC Model

| Role | View | Actions |
|------|------|---------|
| **Viewer** | All dashboards | None |
| **Operator** | All dashboards | Safe actions (low/medium risk) |
| **Admin** | All dashboards + audit logs | All actions including high risk |

---

## 7. Aggregation Pipeline

### 7.1 Continuous Rollup Jobs

```sql
-- Run every 5 minutes to update hourly aggregates
INSERT INTO agg.impressions_hourly (
    bucket_hour, tenant_id, app_id, placement_id, adapter_id,
    country_code, ad_format,
    impression_count, filled_count, revenue_micros, error_count,
    latency_sum_ms, latency_count
)
SELECT
    DATE_TRUNC('hour', event_time) AS bucket_hour,
    tenant_id, app_id, placement_id, adapter_id,
    country_code, 
    ad_format,
    COUNT(*) AS impression_count,
    COUNT(*) FILTER (WHERE filled) AS filled_count,
    SUM(revenue_micros) AS revenue_micros,
    0 AS error_count,
    SUM(latency_ms) AS latency_sum_ms,
    COUNT(latency_ms) AS latency_count
FROM events.impressions
WHERE event_time >= NOW() - INTERVAL '1 hour'
GROUP BY 1,2,3,4,5,6,7
ON CONFLICT (bucket_hour, tenant_id, app_id, placement_id, adapter_id, country_code, ad_format)
DO UPDATE SET
    impression_count = EXCLUDED.impression_count,
    filled_count = EXCLUDED.filled_count,
    revenue_micros = EXCLUDED.revenue_micros,
    latency_sum_ms = EXCLUDED.latency_sum_ms,
    latency_count = EXCLUDED.latency_count,
    updated_at = NOW();
```

### 7.2 Dashboard Query Pattern

```sql
-- Dashboard never touches raw events, only agg tables
SELECT 
    bucket_date,
    SUM(impression_count) AS impressions,
    SUM(revenue_micros)::NUMERIC / 1000000 AS revenue_usd,
    AVG(fill_rate) AS avg_fill_rate,
    AVG(ecpm_micros)::NUMERIC / 1000000 AS avg_ecpm
FROM agg.impressions_daily
WHERE tenant_id = $1
  AND bucket_date >= CURRENT_DATE - INTERVAL '30 days'
GROUP BY bucket_date
ORDER BY bucket_date;
```

---

## 8. Implementation Checklist

### Phase 1: DB Server Setup
- [ ] Configure private network only (no public IP)
- [ ] Install PostgreSQL 16 with pg_stat_statements
- [ ] Apply postgresql.conf tuning
- [ ] Create schemas (core, config, ops, events, agg)
- [ ] Run all schema migrations
- [ ] Install Autopilot service
- [ ] Configure firewall (only accept from App Server IP)

### Phase 2: App Server Setup  
- [ ] Install Docker + Docker Compose
- [ ] Deploy Backend container
- [ ] Deploy Console container
- [ ] Deploy Admin Control Panel container
- [ ] Configure Redis
- [ ] Configure Nginx with SSL
- [ ] Connect to DB via private network

### Phase 3: Admin Control Panel
- [ ] Build Control Panel UI (same style as Console)
- [ ] Wire to Autopilot API
- [ ] Wire to Backend API
- [ ] Implement RBAC
- [ ] Add action audit logging

### Phase 4: Go-Live Preparation
- [ ] Run load test
- [ ] Verify partition creation/drop
- [ ] Test failover scenarios
- [ ] Document runbooks
- [ ] DNS cutover (not yet)

---

## 9. Hetzner Server Access

| Server | Role | Public IP | Private IP | SSH Command |
|--------|------|-----------|------------|-------------|
| **App Server** | Backend + Console + Admin | 77.42.44.35 | TBD | `ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35` |
| **DB Server** | PostgreSQL + Autopilot | NONE | TBD | Via App Server jump host only |

### SSH to DB Server (via jump host)
```bash
# From local machine
ssh -i ~/.ssh/hetzner-db-mac -J root@77.42.44.35 root@<DB_PRIVATE_IP>

# Or configure ~/.ssh/config
Host hetzner-app
    HostName 77.42.44.35
    User root
    IdentityFile ~/.ssh/hetzner-db-mac

Host hetzner-db
    HostName <DB_PRIVATE_IP>
    User root
    IdentityFile ~/.ssh/hetzner-db-mac
    ProxyJump hetzner-app
```
