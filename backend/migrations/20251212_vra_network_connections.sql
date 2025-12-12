-- VRA Network Connections Migration
-- Enables publishers to connect ad networks for automated reconciliation

-- Network credentials table (encrypted at rest)
CREATE TABLE IF NOT EXISTS vra_network_connections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES publishers(id) ON DELETE CASCADE,
    network_type VARCHAR(50) NOT NULL, -- 'admob', 'unity', 'applovin', 'ironsource', 'meta', 'vungle'
    display_name VARCHAR(255),
    
    -- Connection status
    status VARCHAR(20) NOT NULL DEFAULT 'pending', -- 'pending', 'active', 'error', 'disabled'
    status_message TEXT,
    last_sync_at TIMESTAMPTZ,
    last_error_at TIMESTAMPTZ,
    
    -- Encrypted credentials (use pgcrypto or application-level encryption)
    credentials_encrypted BYTEA, -- JSON blob encrypted with AES-256
    credentials_iv BYTEA, -- IV for AES decryption
    
    -- OAuth tokens (for networks that use OAuth)
    oauth_access_token TEXT,
    oauth_refresh_token TEXT,
    oauth_expires_at TIMESTAMPTZ,
    
    -- Network-specific identifiers
    network_account_id VARCHAR(255), -- e.g., AdMob publisher ID
    network_app_ids JSONB DEFAULT '[]', -- array of app IDs linked
    
    -- Sync configuration
    sync_enabled BOOLEAN DEFAULT true,
    sync_frequency_hours INTEGER DEFAULT 24,
    sync_lookback_days INTEGER DEFAULT 3,
    
    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(publisher_id, network_type)
);

-- Index for efficient lookups
CREATE INDEX IF NOT EXISTS idx_vra_network_connections_publisher 
    ON vra_network_connections(publisher_id);
CREATE INDEX IF NOT EXISTS idx_vra_network_connections_status 
    ON vra_network_connections(status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_vra_network_connections_sync 
    ON vra_network_connections(last_sync_at) WHERE sync_enabled = true;

-- Network sync history for debugging and audit
CREATE TABLE IF NOT EXISTS vra_sync_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    connection_id UUID NOT NULL REFERENCES vra_network_connections(id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status VARCHAR(20) NOT NULL DEFAULT 'running', -- 'running', 'success', 'partial', 'error'
    
    -- Sync window
    sync_from DATE NOT NULL,
    sync_to DATE NOT NULL,
    
    -- Results
    rows_fetched INTEGER DEFAULT 0,
    rows_inserted INTEGER DEFAULT 0,
    rows_updated INTEGER DEFAULT 0,
    revenue_usd_total DECIMAL(15,6) DEFAULT 0,
    
    -- Error tracking
    error_message TEXT,
    error_details JSONB,
    
    -- Performance
    duration_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_vra_sync_history_connection 
    ON vra_sync_history(connection_id, started_at DESC);

-- Reconciliation runs table
CREATE TABLE IF NOT EXISTS vra_reconciliation_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES publishers(id),
    
    -- Run window
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    
    -- Status
    status VARCHAR(20) NOT NULL DEFAULT 'running', -- 'running', 'completed', 'error'
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    
    -- Results
    expected_usd DECIMAL(15,6) DEFAULT 0,
    paid_usd DECIMAL(15,6) DEFAULT 0,
    variance_usd DECIMAL(15,6) DEFAULT 0,
    variance_percent DECIMAL(8,4) DEFAULT 0,
    coverage_percent DECIMAL(8,4) DEFAULT 0,
    
    -- Deltas generated
    deltas_count INTEGER DEFAULT 0,
    deltas_by_kind JSONB DEFAULT '{}',
    
    -- Metadata
    triggered_by VARCHAR(50) DEFAULT 'scheduled', -- 'scheduled', 'manual', 'webhook'
    duration_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_vra_recon_runs_publisher 
    ON vra_reconciliation_runs(publisher_id, started_at DESC);

-- Variance alerts table
CREATE TABLE IF NOT EXISTS vra_alerts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES publishers(id),
    reconciliation_run_id UUID REFERENCES vra_reconciliation_runs(id),
    
    -- Alert details
    alert_type VARCHAR(50) NOT NULL, -- 'high_variance', 'missing_data', 'sync_failure', 'underpay_detected'
    severity VARCHAR(20) NOT NULL DEFAULT 'warning', -- 'info', 'warning', 'critical'
    
    -- Context
    network_type VARCHAR(50),
    amount_usd DECIMAL(15,6),
    message TEXT NOT NULL,
    details JSONB,
    
    -- Status
    status VARCHAR(20) NOT NULL DEFAULT 'open', -- 'open', 'acknowledged', 'resolved', 'dismissed'
    acknowledged_at TIMESTAMPTZ,
    acknowledged_by UUID REFERENCES users(id),
    resolved_at TIMESTAMPTZ,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_vra_alerts_publisher_status 
    ON vra_alerts(publisher_id, status, created_at DESC);

-- Ensure recon tables exist with proper schema
CREATE TABLE IF NOT EXISTS recon_expected (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL,
    request_id VARCHAR(255),
    network_id VARCHAR(100),
    app_id VARCHAR(255),
    ad_unit_id VARCHAR(255),
    country_code VARCHAR(10),
    ad_format VARCHAR(50),
    ts TIMESTAMPTZ NOT NULL,
    period_start DATE,
    period_end DATE,
    expected_value DECIMAL(15,6) NOT NULL DEFAULT 0,
    currency VARCHAR(10) DEFAULT 'USD',
    viewability JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_recon_expected_ts ON recon_expected(ts);
CREATE INDEX IF NOT EXISTS idx_recon_expected_publisher ON recon_expected(publisher_id, ts);
CREATE INDEX IF NOT EXISTS idx_recon_expected_request ON recon_expected(request_id) WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS recon_statements_norm (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL,
    connection_id UUID REFERENCES vra_network_connections(id),
    network_type VARCHAR(50) NOT NULL,
    
    -- Statement identifiers
    statement_id VARCHAR(255), -- network's unique ID if available
    event_date DATE NOT NULL,
    
    -- Dimensions
    app_id VARCHAR(255),
    ad_unit_id VARCHAR(255),
    country VARCHAR(10),
    format VARCHAR(50),
    
    -- Metrics (normalized to USD)
    impressions BIGINT DEFAULT 0,
    clicks BIGINT DEFAULT 0,
    paid DECIMAL(15,6) NOT NULL DEFAULT 0, -- net revenue in USD
    gross DECIMAL(15,6) DEFAULT 0,
    
    -- Adjustments
    ivt_adjustments DECIMAL(15,6) DEFAULT 0,
    exchange_rate DECIMAL(15,8) DEFAULT 1,
    original_currency VARCHAR(10),
    original_amount DECIMAL(15,6),
    
    -- Metadata
    raw_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(publisher_id, network_type, event_date, app_id, ad_unit_id, country, format)
);

CREATE INDEX IF NOT EXISTS idx_recon_statements_date ON recon_statements_norm(event_date);
CREATE INDEX IF NOT EXISTS idx_recon_statements_publisher ON recon_statements_norm(publisher_id, event_date);

-- Ensure recon_deltas has all needed columns
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS publisher_id UUID;
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS reconciliation_run_id UUID;
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS network VARCHAR(100);
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS app_id VARCHAR(255);
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS resolved BOOLEAN DEFAULT false;
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS resolved_at TIMESTAMPTZ;
ALTER TABLE recon_deltas ADD COLUMN IF NOT EXISTS dispute_id UUID;

CREATE INDEX IF NOT EXISTS idx_recon_deltas_publisher ON recon_deltas(publisher_id, window_start);

-- Function to update timestamps
CREATE OR REPLACE FUNCTION update_vra_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS update_vra_network_connections_timestamp ON vra_network_connections;
CREATE TRIGGER update_vra_network_connections_timestamp
    BEFORE UPDATE ON vra_network_connections
    FOR EACH ROW
    EXECUTE FUNCTION update_vra_updated_at();
