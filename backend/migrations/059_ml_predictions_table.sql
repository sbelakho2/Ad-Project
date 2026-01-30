-- Migration: ML Predictions Table for Model Training Feedback Loop
-- This table stores ML model predictions with outcomes and feedback labels
-- for model performance monitoring and retraining pipelines.

BEGIN;

-- Main ML predictions table
CREATE TABLE IF NOT EXISTS ml_predictions (
    prediction_id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    model_name VARCHAR(64) NOT NULL,
    model_version VARCHAR(32) NOT NULL,
    
    -- Prediction details
    raw_score FLOAT NOT NULL,
    normalized_score FLOAT NOT NULL,
    threshold FLOAT NOT NULL,
    decision VARCHAR(16) NOT NULL CHECK (decision IN ('allow', 'flag', 'block')),
    confidence FLOAT NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    
    -- Features (for debugging and retraining)
    features JSONB NOT NULL DEFAULT '{}',
    feature_importance JSONB,
    
    -- Context (hashed for privacy)
    publisher_id UUID,
    placement_id UUID,
    device_fingerprint_hash VARCHAR(32),
    ip_hash VARCHAR(32),
    
    -- Timing
    inference_latency_ms FLOAT NOT NULL,
    predicted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- ML experiment tracking
    is_canary BOOLEAN DEFAULT FALSE,
    experiment_id VARCHAR(64),
    reasons JSONB DEFAULT '[]',
    
    -- Outcome tracking (filled later via feedback loop)
    outcome_type VARCHAR(32), -- impression, click, install, conversion, chargeback, none
    outcome_timestamp TIMESTAMPTZ,
    outcome_value FLOAT,
    
    -- Feedback/labels (filled by manual review or automated systems)
    feedback_type VARCHAR(32), -- confirmed_fraud, confirmed_legitimate, chargeback, user_report, manual_review
    was_correct BOOLEAN,
    feedback_timestamp TIMESTAMPTZ,
    feedback_source VARCHAR(64),
    feedback_metadata JSONB
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_ml_predictions_model_time 
    ON ml_predictions (model_name, model_version, predicted_at DESC);

CREATE INDEX IF NOT EXISTS idx_ml_predictions_publisher 
    ON ml_predictions (publisher_id, predicted_at DESC) 
    WHERE publisher_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_ml_predictions_decision 
    ON ml_predictions (decision, predicted_at DESC);

CREATE INDEX IF NOT EXISTS idx_ml_predictions_feedback 
    ON ml_predictions (was_correct, predicted_at DESC) 
    WHERE was_correct IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_ml_predictions_experiment 
    ON ml_predictions (experiment_id, predicted_at DESC) 
    WHERE experiment_id IS NOT NULL;

-- Index for training data export queries
CREATE INDEX IF NOT EXISTS idx_ml_predictions_training_export
    ON ml_predictions (model_name, predicted_at)
    WHERE was_correct IS NOT NULL OR outcome_type IS NOT NULL;

-- Partitioning by time for efficient data retention
-- Note: For production, consider partitioning by predicted_at month
COMMENT ON TABLE ml_predictions IS 
    'ML model predictions with outcomes and feedback labels for model training feedback loop. 
     Supports model performance monitoring, drift detection, and retraining pipelines.';

-- Aggregated model performance view for dashboards
CREATE OR REPLACE VIEW ml_model_performance_summary AS
SELECT 
    model_name,
    model_version,
    DATE_TRUNC('hour', predicted_at) as hour,
    COUNT(*) as total_predictions,
    COUNT(*) FILTER (WHERE was_correct IS NOT NULL) as labeled_predictions,
    COUNT(*) FILTER (WHERE was_correct = true) as correct_predictions,
    COUNT(*) FILTER (WHERE was_correct = false) as incorrect_predictions,
    AVG(inference_latency_ms) as avg_latency_ms,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY inference_latency_ms) as p95_latency_ms,
    AVG(raw_score) as avg_score,
    COUNT(*) FILTER (WHERE decision = 'block') as block_count,
    COUNT(*) FILTER (WHERE decision = 'flag') as flag_count,
    COUNT(*) FILTER (WHERE decision = 'allow') as allow_count
FROM ml_predictions
GROUP BY model_name, model_version, DATE_TRUNC('hour', predicted_at);

-- Confusion matrix view for model evaluation
CREATE OR REPLACE VIEW ml_model_confusion_matrix AS
SELECT 
    model_name,
    model_version,
    DATE_TRUNC('day', predicted_at) as day,
    COUNT(*) FILTER (WHERE decision = 'block' AND was_correct = true) as true_positives,
    COUNT(*) FILTER (WHERE decision = 'block' AND was_correct = false) as false_positives,
    COUNT(*) FILTER (WHERE decision = 'allow' AND was_correct = true) as true_negatives,
    COUNT(*) FILTER (WHERE decision = 'allow' AND was_correct = false) as false_negatives,
    COUNT(*) FILTER (WHERE was_correct IS NOT NULL) as total_labeled
FROM ml_predictions
WHERE was_correct IS NOT NULL
GROUP BY model_name, model_version, DATE_TRUNC('day', predicted_at);

-- Data retention policy: Keep detailed data for 90 days, aggregate older data
-- This function can be called by a scheduled job
CREATE OR REPLACE FUNCTION cleanup_old_ml_predictions(retention_days INTEGER DEFAULT 90)
RETURNS INTEGER AS $$
DECLARE
    deleted_count INTEGER;
BEGIN
    DELETE FROM ml_predictions 
    WHERE predicted_at < NOW() - (retention_days || ' days')::INTERVAL;
    
    GET DIAGNOSTICS deleted_count = ROW_COUNT;
    RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION cleanup_old_ml_predictions IS 
    'Removes ML predictions older than the specified retention period. 
     Call periodically (e.g., daily) to manage table size.';

COMMIT;
