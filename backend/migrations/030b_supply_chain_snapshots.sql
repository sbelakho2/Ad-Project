-- Persist per-publisher supply chain summaries for Console and auditing
CREATE TABLE IF NOT EXISTS supply_chain_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    publisher_id UUID NOT NULL REFERENCES publishers(id) ON DELETE CASCADE,
    summary JSONB NOT NULL,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_supply_chain_snapshots_publisher_generated_at
    ON supply_chain_snapshots (publisher_id, generated_at DESC);
