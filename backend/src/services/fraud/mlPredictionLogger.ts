/**
 * ML Prediction Logger Service
 * 
 * Comprehensive logging for ML model predictions to support:
 * 1. Model performance tracking and drift detection
 * 2. Feedback loop for model retraining
 * 3. A/B testing of model versions
 * 4. Compliance and audit trails
 * 
 * Design principles:
 * - Non-blocking: all logging is async and fire-and-forget
 * - Batch-optimized: buffers predictions and flushes periodically
 * - Privacy-aware: hashes PII before storage
 * - Structured: JSON format suitable for ML pipelines
 */

import { query } from '../../utils/postgres';
import logger from '../../utils/logger';
import { Counter, Histogram, Gauge } from 'prom-client';
import crypto from 'crypto';

// -----------------------------------------------------
// Prometheus Metrics
// -----------------------------------------------------

export const mlPredictionLoggedTotal = new Counter({
  name: 'ml_prediction_logged_total',
  help: 'Total ML predictions logged',
  labelNames: ['model_name', 'model_version', 'prediction_type'],
});

export const mlPredictionBufferSize = new Gauge({
  name: 'ml_prediction_buffer_size',
  help: 'Current size of the prediction buffer',
  labelNames: ['model_name'],
});

export const mlPredictionFlushDuration = new Histogram({
  name: 'ml_prediction_flush_duration_seconds',
  help: 'Duration of batch flush operations',
  buckets: [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5],
  labelNames: ['model_name'],
});

export const mlFeedbackReceivedTotal = new Counter({
  name: 'ml_feedback_received_total',
  help: 'Total feedback labels received for predictions',
  labelNames: ['model_name', 'feedback_type', 'was_correct'],
});

// -----------------------------------------------------
// Types
// -----------------------------------------------------

export interface MLPrediction {
  // Identifiers
  predictionId: string;
  requestId: string;
  modelName: string;
  modelVersion: string;
  
  // Prediction details
  rawScore: number;
  normalizedScore: number;
  threshold: number;
  decision: 'allow' | 'flag' | 'block';
  confidence: number;
  
  // Features used (for model debugging)
  features: Record<string, number>;
  featureImportance?: Record<string, number>;
  
  // Context
  publisherId?: string;
  placementId?: string;
  deviceFingerprintHash?: string;
  ipHash?: string;
  
  // Timing
  inferenceLatencyMs: number;
  timestamp: Date;
  
  // ML-specific
  isCanary?: boolean;
  experimentId?: string;
  reasons?: string[];
}

export interface MLFeedback {
  predictionId: string;
  feedbackType: 'confirmed_fraud' | 'confirmed_legitimate' | 'chargeback' | 'user_report' | 'manual_review';
  wasCorrect: boolean;
  feedbackTimestamp: Date;
  feedbackSource: string;
  metadata?: Record<string, unknown>;
}

export interface PredictionOutcome {
  predictionId: string;
  outcomeType: 'impression' | 'click' | 'install' | 'conversion' | 'chargeback' | 'none';
  outcomeTimestamp?: Date;
  outcomeValue?: number;
}

// -----------------------------------------------------
// Configuration
// -----------------------------------------------------

const BUFFER_FLUSH_INTERVAL_MS = parseInt(process.env.ML_LOG_FLUSH_INTERVAL_MS || '5000', 10);
const BUFFER_MAX_SIZE = parseInt(process.env.ML_LOG_BUFFER_MAX_SIZE || '500', 10);
const ENABLE_ML_LOGGING = process.env.ENABLE_ML_PREDICTION_LOGGING !== 'false';

// -----------------------------------------------------
// Buffer Management
// -----------------------------------------------------

interface BufferEntry {
  prediction: MLPrediction;
  outcome?: PredictionOutcome;
  feedback?: MLFeedback;
}

const predictionBuffer: Map<string, BufferEntry> = new Map();
let flushTimer: NodeJS.Timeout | null = null;
let isShuttingDown = false;

/**
 * Hash PII fields for privacy-compliant logging
 */
function hashPII(value: string | undefined): string | undefined {
  if (!value) return undefined;
  return crypto.createHash('sha256').update(value).digest('hex').substring(0, 16);
}

/**
 * Log an ML prediction (buffered, async)
 */
export function logPrediction(prediction: MLPrediction): void {
  if (!ENABLE_ML_LOGGING) return;
  
  const sanitized: MLPrediction = {
    ...prediction,
    deviceFingerprintHash: prediction.deviceFingerprintHash 
      ? hashPII(prediction.deviceFingerprintHash) 
      : undefined,
    ipHash: prediction.ipHash 
      ? hashPII(prediction.ipHash) 
      : undefined,
  };
  
  predictionBuffer.set(prediction.predictionId, { prediction: sanitized });
  
  mlPredictionLoggedTotal.inc({
    model_name: prediction.modelName,
    model_version: prediction.modelVersion,
    prediction_type: prediction.decision,
  });
  
  mlPredictionBufferSize.set({ model_name: prediction.modelName }, predictionBuffer.size);
  
  // Flush if buffer is full
  if (predictionBuffer.size >= BUFFER_MAX_SIZE) {
    void flushBuffer();
  }
  
  // Ensure flush timer is running
  ensureFlushTimer();
}

/**
 * Record outcome for a prediction (for feedback loop)
 */
export function recordOutcome(outcome: PredictionOutcome): void {
  if (!ENABLE_ML_LOGGING) return;
  
  const entry = predictionBuffer.get(outcome.predictionId);
  if (entry) {
    entry.outcome = outcome;
  } else {
    // Prediction was already flushed, update directly in DB
    void updateOutcomeInDB(outcome);
  }
}

/**
 * Record feedback/label for a prediction (for model retraining)
 */
export function recordFeedback(feedback: MLFeedback): void {
  if (!ENABLE_ML_LOGGING) return;
  
  mlFeedbackReceivedTotal.inc({
    model_name: 'fraud_detection', // Could be derived from prediction
    feedback_type: feedback.feedbackType,
    was_correct: feedback.wasCorrect ? 'true' : 'false',
  });
  
  const entry = predictionBuffer.get(feedback.predictionId);
  if (entry) {
    entry.feedback = feedback;
  } else {
    // Prediction was already flushed, update directly in DB
    void updateFeedbackInDB(feedback);
  }
}

/**
 * Ensure the flush timer is running
 */
function ensureFlushTimer(): void {
  if (flushTimer === null && !isShuttingDown) {
    flushTimer = setInterval(() => {
      void flushBuffer();
    }, BUFFER_FLUSH_INTERVAL_MS);
  }
}

/**
 * Flush the prediction buffer to the database
 */
export async function flushBuffer(): Promise<void> {
  if (predictionBuffer.size === 0) return;
  
  const startTime = Date.now();
  const entries = Array.from(predictionBuffer.values());
  predictionBuffer.clear();
  
  // Group by model for metrics
  const modelCounts: Record<string, number> = {};
  for (const entry of entries) {
    modelCounts[entry.prediction.modelName] = (modelCounts[entry.prediction.modelName] || 0) + 1;
  }
  
  try {
    // Batch insert predictions
    const values: unknown[][] = entries.map(entry => [
      entry.prediction.predictionId,
      entry.prediction.requestId,
      entry.prediction.modelName,
      entry.prediction.modelVersion,
      entry.prediction.rawScore,
      entry.prediction.normalizedScore,
      entry.prediction.threshold,
      entry.prediction.decision,
      entry.prediction.confidence,
      JSON.stringify(entry.prediction.features),
      JSON.stringify(entry.prediction.featureImportance || null),
      entry.prediction.publisherId || null,
      entry.prediction.placementId || null,
      entry.prediction.deviceFingerprintHash || null,
      entry.prediction.ipHash || null,
      entry.prediction.inferenceLatencyMs,
      entry.prediction.timestamp,
      entry.prediction.isCanary || false,
      entry.prediction.experimentId || null,
      JSON.stringify(entry.prediction.reasons || []),
      entry.outcome?.outcomeType || null,
      entry.outcome?.outcomeTimestamp || null,
      entry.outcome?.outcomeValue || null,
      entry.feedback?.feedbackType || null,
      entry.feedback?.wasCorrect ?? null,
      entry.feedback?.feedbackTimestamp || null,
      entry.feedback?.feedbackSource || null,
      JSON.stringify(entry.feedback?.metadata || null),
    ]);
    
    // Use unnest for efficient batch insert
    await query(
      `INSERT INTO ml_predictions (
        prediction_id, request_id, model_name, model_version,
        raw_score, normalized_score, threshold, decision, confidence,
        features, feature_importance,
        publisher_id, placement_id, device_fingerprint_hash, ip_hash,
        inference_latency_ms, predicted_at,
        is_canary, experiment_id, reasons,
        outcome_type, outcome_timestamp, outcome_value,
        feedback_type, was_correct, feedback_timestamp, feedback_source, feedback_metadata
      )
      SELECT * FROM unnest(
        $1::uuid[], $2::uuid[], $3::text[], $4::text[],
        $5::float[], $6::float[], $7::float[], $8::text[], $9::float[],
        $10::jsonb[], $11::jsonb[],
        $12::uuid[], $13::uuid[], $14::text[], $15::text[],
        $16::float[], $17::timestamptz[],
        $18::boolean[], $19::text[], $20::jsonb[],
        $21::text[], $22::timestamptz[], $23::float[],
        $24::text[], $25::boolean[], $26::timestamptz[], $27::text[], $28::jsonb[]
      )
      ON CONFLICT (prediction_id) DO UPDATE SET
        outcome_type = COALESCE(EXCLUDED.outcome_type, ml_predictions.outcome_type),
        outcome_timestamp = COALESCE(EXCLUDED.outcome_timestamp, ml_predictions.outcome_timestamp),
        outcome_value = COALESCE(EXCLUDED.outcome_value, ml_predictions.outcome_value),
        feedback_type = COALESCE(EXCLUDED.feedback_type, ml_predictions.feedback_type),
        was_correct = COALESCE(EXCLUDED.was_correct, ml_predictions.was_correct),
        feedback_timestamp = COALESCE(EXCLUDED.feedback_timestamp, ml_predictions.feedback_timestamp),
        feedback_source = COALESCE(EXCLUDED.feedback_source, ml_predictions.feedback_source),
        feedback_metadata = COALESCE(EXCLUDED.feedback_metadata, ml_predictions.feedback_metadata)`,
      [
        values.map(v => v[0]),  // prediction_id
        values.map(v => v[1]),  // request_id
        values.map(v => v[2]),  // model_name
        values.map(v => v[3]),  // model_version
        values.map(v => v[4]),  // raw_score
        values.map(v => v[5]),  // normalized_score
        values.map(v => v[6]),  // threshold
        values.map(v => v[7]),  // decision
        values.map(v => v[8]),  // confidence
        values.map(v => v[9]),  // features
        values.map(v => v[10]), // feature_importance
        values.map(v => v[11]), // publisher_id
        values.map(v => v[12]), // placement_id
        values.map(v => v[13]), // device_fingerprint_hash
        values.map(v => v[14]), // ip_hash
        values.map(v => v[15]), // inference_latency_ms
        values.map(v => v[16]), // predicted_at
        values.map(v => v[17]), // is_canary
        values.map(v => v[18]), // experiment_id
        values.map(v => v[19]), // reasons
        values.map(v => v[20]), // outcome_type
        values.map(v => v[21]), // outcome_timestamp
        values.map(v => v[22]), // outcome_value
        values.map(v => v[23]), // feedback_type
        values.map(v => v[24]), // was_correct
        values.map(v => v[25]), // feedback_timestamp
        values.map(v => v[26]), // feedback_source
        values.map(v => v[27]), // feedback_metadata
      ]
    );
    
    const durationSec = (Date.now() - startTime) / 1000;
    for (const [modelName, count] of Object.entries(modelCounts)) {
      mlPredictionFlushDuration.observe({ model_name: modelName }, durationSec);
      mlPredictionBufferSize.set({ model_name: modelName }, 0);
      logger.debug('Flushed ML predictions', { modelName, count, durationMs: durationSec * 1000 });
    }
  } catch (error) {
    logger.error('Failed to flush ML predictions', { error, count: entries.length });
    // Re-add entries to buffer for retry (but limit to prevent memory issues)
    if (predictionBuffer.size < BUFFER_MAX_SIZE * 2) {
      for (const entry of entries.slice(0, BUFFER_MAX_SIZE)) {
        predictionBuffer.set(entry.prediction.predictionId, entry);
      }
    }
  }
}

/**
 * Update outcome directly in DB (for predictions already flushed)
 */
async function updateOutcomeInDB(outcome: PredictionOutcome): Promise<void> {
  try {
    await query(
      `UPDATE ml_predictions SET
        outcome_type = $2,
        outcome_timestamp = $3,
        outcome_value = $4
       WHERE prediction_id = $1`,
      [outcome.predictionId, outcome.outcomeType, outcome.outcomeTimestamp, outcome.outcomeValue]
    );
  } catch (error) {
    logger.debug('Failed to update outcome in DB', { error, predictionId: outcome.predictionId });
  }
}

/**
 * Update feedback directly in DB (for predictions already flushed)
 */
async function updateFeedbackInDB(feedback: MLFeedback): Promise<void> {
  try {
    await query(
      `UPDATE ml_predictions SET
        feedback_type = $2,
        was_correct = $3,
        feedback_timestamp = $4,
        feedback_source = $5,
        feedback_metadata = $6
       WHERE prediction_id = $1`,
      [
        feedback.predictionId,
        feedback.feedbackType,
        feedback.wasCorrect,
        feedback.feedbackTimestamp,
        feedback.feedbackSource,
        JSON.stringify(feedback.metadata || null),
      ]
    );
    
    mlFeedbackReceivedTotal.inc({
      model_name: 'fraud_detection',
      feedback_type: feedback.feedbackType,
      was_correct: feedback.wasCorrect ? 'true' : 'false',
    });
  } catch (error) {
    logger.debug('Failed to update feedback in DB', { error, predictionId: feedback.predictionId });
  }
}

// -----------------------------------------------------
// Training Data Export
// -----------------------------------------------------

export interface TrainingDataExportOptions {
  modelName: string;
  startDate: Date;
  endDate: Date;
  minFeedbackCount?: number;
  includeFeatures?: boolean;
  limit?: number;
}

export interface TrainingDataRow {
  predictionId: string;
  features: Record<string, number>;
  rawScore: number;
  decision: string;
  outcomeType?: string;
  wasCorrect?: boolean;
  feedbackType?: string;
  predictedAt: Date;
}

/**
 * Export labeled training data for model retraining
 */
export async function exportTrainingData(options: TrainingDataExportOptions): Promise<TrainingDataRow[]> {
  const {
    modelName,
    startDate,
    endDate,
    minFeedbackCount = 100,
    includeFeatures = true,
    limit = 100000,
  } = options;
  
  const result = await query(
    `SELECT 
      prediction_id,
      ${includeFeatures ? 'features,' : ''}
      raw_score,
      decision,
      outcome_type,
      was_correct,
      feedback_type,
      predicted_at
     FROM ml_predictions
     WHERE model_name = $1
       AND predicted_at BETWEEN $2 AND $3
       AND (was_correct IS NOT NULL OR outcome_type IS NOT NULL)
     ORDER BY predicted_at DESC
     LIMIT $4`,
    [modelName, startDate, endDate, limit]
  );
  
  return result.rows.map(row => ({
    predictionId: row.prediction_id,
    features: includeFeatures ? row.features : {},
    rawScore: parseFloat(row.raw_score),
    decision: row.decision,
    outcomeType: row.outcome_type,
    wasCorrect: row.was_correct,
    feedbackType: row.feedback_type,
    predictedAt: row.predicted_at,
  }));
}

// -----------------------------------------------------
// Model Performance Metrics
// -----------------------------------------------------

export interface ModelPerformanceMetrics {
  modelName: string;
  modelVersion: string;
  timeWindow: string;
  totalPredictions: number;
  truePositives: number;
  falsePositives: number;
  trueNegatives: number;
  falseNegatives: number;
  precision: number;
  recall: number;
  f1Score: number;
  accuracy: number;
  avgLatencyMs: number;
  p95LatencyMs: number;
}

/**
 * Calculate model performance metrics from logged predictions with feedback
 */
export async function getModelPerformance(
  modelName: string,
  modelVersion: string,
  windowHours: number = 24
): Promise<ModelPerformanceMetrics | null> {
  const result = await query(
    `WITH stats AS (
      SELECT
        COUNT(*) as total,
        SUM(CASE WHEN decision = 'block' AND was_correct = true THEN 1 ELSE 0 END) as tp,
        SUM(CASE WHEN decision = 'block' AND was_correct = false THEN 1 ELSE 0 END) as fp,
        SUM(CASE WHEN decision = 'allow' AND was_correct = true THEN 1 ELSE 0 END) as tn,
        SUM(CASE WHEN decision = 'allow' AND was_correct = false THEN 1 ELSE 0 END) as fn,
        AVG(inference_latency_ms) as avg_latency,
        PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY inference_latency_ms) as p95_latency
      FROM ml_predictions
      WHERE model_name = $1
        AND model_version = $2
        AND predicted_at >= NOW() - INTERVAL '1 hour' * $3
        AND was_correct IS NOT NULL
    )
    SELECT * FROM stats`,
    [modelName, modelVersion, windowHours]
  );
  
  if (!result.rows[0] || result.rows[0].total === 0) {
    return null;
  }
  
  const row = result.rows[0];
  const tp = parseInt(row.tp) || 0;
  const fp = parseInt(row.fp) || 0;
  const tn = parseInt(row.tn) || 0;
  const fn = parseInt(row.fn) || 0;
  const total = parseInt(row.total) || 0;
  
  const precision = tp + fp > 0 ? tp / (tp + fp) : 0;
  const recall = tp + fn > 0 ? tp / (tp + fn) : 0;
  const f1Score = precision + recall > 0 ? (2 * precision * recall) / (precision + recall) : 0;
  const accuracy = total > 0 ? (tp + tn) / total : 0;
  
  return {
    modelName,
    modelVersion,
    timeWindow: `${windowHours}h`,
    totalPredictions: total,
    truePositives: tp,
    falsePositives: fp,
    trueNegatives: tn,
    falseNegatives: fn,
    precision: Math.round(precision * 1000) / 1000,
    recall: Math.round(recall * 1000) / 1000,
    f1Score: Math.round(f1Score * 1000) / 1000,
    accuracy: Math.round(accuracy * 1000) / 1000,
    avgLatencyMs: parseFloat(row.avg_latency) || 0,
    p95LatencyMs: parseFloat(row.p95_latency) || 0,
  };
}

// -----------------------------------------------------
// Graceful Shutdown
// -----------------------------------------------------

export async function shutdown(): Promise<void> {
  isShuttingDown = true;
  if (flushTimer) {
    clearInterval(flushTimer);
    flushTimer = null;
  }
  await flushBuffer();
}

// Register shutdown handler
process.on('SIGTERM', () => void shutdown());
process.on('SIGINT', () => void shutdown());

export default {
  logPrediction,
  recordOutcome,
  recordFeedback,
  flushBuffer,
  exportTrainingData,
  getModelPerformance,
  shutdown,
};
