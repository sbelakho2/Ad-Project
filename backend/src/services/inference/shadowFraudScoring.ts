/**
 * Shadow Fraud Scoring Service
 * 
 * Scores auction requests for fraud risk WITHOUT blocking traffic.
 * All scores are logged to analytics only for model training and drift detection.
 * This follows the SDK_CHECKS requirement: "Shadow fraud scoring ON; never block"
 * 
 * Design:
 * - Non-blocking: scoring happens async, never delays auction path
 * - Analytics-only: scores go to DB/analytics, not used for gating
 * - Drift monitoring: tracks PSI between training and production distributions
 */

import logger from '../../utils/logger';
import { query } from '../../utils/postgres';
import { selectModelVersion, getModelEndpoint } from '../../utils/mlCanary';
import { Counter, Histogram, Gauge } from 'prom-client';
import { sendAlert } from '../alertService';

// -----------------------------------------------------
// Prometheus Metrics for shadow scoring
// -----------------------------------------------------

export const shadowFraudScoreHistogram = new Histogram({
  name: 'shadow_fraud_score',
  help: 'Distribution of shadow fraud scores (0-1)',
  buckets: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
  labelNames: ['model_version', 'risk_bucket'],
});

export const shadowFraudScoringTotal = new Counter({
  name: 'shadow_fraud_scoring_total',
  help: 'Total shadow fraud scoring attempts',
  labelNames: ['model_version', 'outcome'],
});

export const shadowFraudPsiGauge = new Gauge({
  name: 'shadow_fraud_psi',
  help: 'Population Stability Index for fraud score distribution',
  labelNames: ['model_version'],
});

export const shadowFraudDriftGauge = new Gauge({
  name: 'shadow_fraud_drift_detected',
  help: 'Whether drift was detected (1) or not (0)',
  labelNames: ['model_version'],
});

// -----------------------------------------------------
// Types
// -----------------------------------------------------

export interface ShadowScoreInput {
  requestId: string;
  placementId: string;
  publisherId?: string;
  deviceInfo?: {
    ip?: string;
    userAgent?: string;
    platform?: string;
    osVersion?: string;
  };
  userInfo?: {
    limitAdTracking?: boolean;
    advertisingId?: string;
  };
  auctionContext?: {
    adFormat?: string;
    floorCpm?: number;
    bidCount?: number;
    winningCpm?: number;
    latencyMs?: number;
  };
  clickTimeToInstall?: number; // CTIT in seconds for post-attribution scoring
  metadata?: Record<string, unknown>;
}

export interface ShadowScoreResult {
  requestId: string;
  score: number; // 0-1, higher = more suspicious
  riskBucket: 'low' | 'medium' | 'high' | 'critical';
  modelVersion: string;
  features: Record<string, number>;
  reasons: string[];
  scoredAt: Date;
}

// -----------------------------------------------------
// Feature Extraction
// -----------------------------------------------------

export function extractFeatures(input: ShadowScoreInput): Record<string, number> {
  const features: Record<string, number> = {};
  
  // Device features
  if (input.deviceInfo) {
    features['has_ip'] = input.deviceInfo.ip ? 1 : 0;
    features['has_user_agent'] = input.deviceInfo.userAgent ? 1 : 0;
    features['is_mobile'] = ['ios', 'android'].includes(input.deviceInfo.platform?.toLowerCase() || '') ? 1 : 0;
  }
  
  // User privacy features
  if (input.userInfo) {
    features['limit_ad_tracking'] = input.userInfo.limitAdTracking ? 1 : 0;
    features['has_advertising_id'] = input.userInfo.advertisingId ? 1 : 0;
  }
  
  // Auction context features
  if (input.auctionContext) {
    features['floor_cpm'] = input.auctionContext.floorCpm ?? 0;
    features['bid_count'] = input.auctionContext.bidCount ?? 0;
    features['winning_cpm'] = input.auctionContext.winningCpm ?? 0;
    features['latency_ms'] = input.auctionContext.latencyMs ?? 0;
    features['cpm_ratio'] = features['floor_cpm'] > 0 
      ? features['winning_cpm'] / features['floor_cpm'] 
      : 0;
  }
  
  // CTIT analysis (Click-Time-To-Install)
  if (typeof input.clickTimeToInstall === 'number') {
    features['ctit_seconds'] = input.clickTimeToInstall;
    // Extremely fast installs are suspicious
    features['ctit_anomaly'] = input.clickTimeToInstall < 10 ? 1 : 0;
  }
  
  return features;
}

// -----------------------------------------------------
// Heuristic Scoring (fallback when ML service unavailable)
// -----------------------------------------------------

export function heuristicScore(features: Record<string, number>): { score: number; reasons: string[] } {
  let score = 0;
  const reasons: string[] = [];
  
  // CTIT anomaly detection
  if (features['ctit_anomaly'] === 1) {
    score += 0.4;
    reasons.push('extremely_fast_ctit');
  }
  
  // Missing device identifiers
  if (features['has_ip'] === 0 && features['has_user_agent'] === 0) {
    score += 0.2;
    reasons.push('missing_device_fingerprint');
  }
  
  // High CPM with no competition
  if (features['bid_count'] <= 1 && features['winning_cpm'] > 20) {
    score += 0.15;
    reasons.push('suspicious_single_bid_high_cpm');
  }
  
  // Extreme latency (could indicate proxy/bot)
  if (features['latency_ms'] < 5 || features['latency_ms'] > 5000) {
    score += 0.1;
    reasons.push('abnormal_latency');
  }
  
  return { score: Math.min(1, score), reasons };
}

function bucketRisk(score: number): 'low' | 'medium' | 'high' | 'critical' {
  if (score < 0.3) return 'low';
  if (score < 0.6) return 'medium';
  if (score < 0.85) return 'high';
  return 'critical';
}

// -----------------------------------------------------
// Main Scoring Function (non-blocking)
// -----------------------------------------------------

/**
 * Score a request for fraud risk in shadow mode.
 * NEVER blocks the auction - always returns quickly.
 * Scores are logged to analytics for training and drift detection.
 */
export async function scoreShadow(input: ShadowScoreInput): Promise<ShadowScoreResult | null> {
  const scoredAt = new Date();
  const modelSelection = selectModelVersion(input.requestId, 'fraud_detection');
  
  try {
    const features = extractFeatures(input);
    
    // Try ML inference service first (with short timeout)
    let score: number;
    let reasons: string[];
    
    try {
      const mlResult = await callMLInference(features, modelSelection.version);
      score = mlResult.score;
      reasons = mlResult.reasons;
    } catch (e) {
      // Fallback to heuristics if ML service unavailable
      const heuristic = heuristicScore(features);
      score = heuristic.score;
      reasons = [...heuristic.reasons, 'ml_fallback'];
      shadowFraudScoringTotal.inc({ model_version: modelSelection.version, outcome: 'ml_unavailable' });
    }
    
    const riskBucket = bucketRisk(score);
    
    // Record metrics
    shadowFraudScoreHistogram.observe(
      { model_version: modelSelection.version, risk_bucket: riskBucket },
      score
    );
    shadowFraudScoringTotal.inc({ model_version: modelSelection.version, outcome: 'success' });
    
    const result: ShadowScoreResult = {
      requestId: input.requestId,
      score,
      riskBucket,
      modelVersion: modelSelection.version,
      features,
      reasons,
      scoredAt,
    };
    
    // Log to analytics (async, non-blocking)
    void logShadowScore(result, input).catch(err => 
      logger.warn('Failed to log shadow score', { error: err, requestId: input.requestId })
    );
    
    return result;
  } catch (error) {
    shadowFraudScoringTotal.inc({ model_version: modelSelection.version, outcome: 'error' });
    logger.warn('Shadow fraud scoring failed', { error, requestId: input.requestId });
    return null;
  }
}

// -----------------------------------------------------
// ML Inference Client
// -----------------------------------------------------

async function callMLInference(
  features: Record<string, number>,
  modelVersion: string
): Promise<{ score: number; reasons: string[] }> {
  const endpoint = getModelEndpoint('fraud_detection', { version: modelVersion, isCanary: modelVersion !== 'stable' });
  
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 100); // 100ms max
  
  try {
    const response = await fetch(endpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ features }),
      signal: controller.signal,
    });
    
    clearTimeout(timeout);
    
    if (!response.ok) {
      throw new Error(`ML inference failed: ${response.status}`);
    }
    
    const result = (await response.json()) as { score?: number; reasons?: string[] };
    return {
      score: typeof result.score === 'number' ? result.score : 0,
      reasons: Array.isArray(result.reasons) ? result.reasons : [],
    };
  } catch (error) {
    clearTimeout(timeout);
    throw error;
  }
}

// -----------------------------------------------------
// Analytics Logging
// -----------------------------------------------------

async function logShadowScore(result: ShadowScoreResult, input: ShadowScoreInput): Promise<void> {
  try {
    await query(
      `INSERT INTO shadow_fraud_scores (
        request_id, placement_id, publisher_id, score, risk_bucket,
        model_version, features, reasons, device_platform, scored_at
      ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
      ON CONFLICT (request_id) DO NOTHING`,
      [
        result.requestId,
        input.placementId,
        input.publisherId || null,
        result.score,
        result.riskBucket,
        result.modelVersion,
        JSON.stringify(result.features),
        JSON.stringify(result.reasons),
        input.deviceInfo?.platform || null,
        result.scoredAt,
      ]
    );
  } catch (error) {
    // Silent fail - analytics logging should never impact main path
    logger.debug('Shadow score logging failed', { error, requestId: result.requestId });
  }
}

// -----------------------------------------------------
// Drift Detection (PSI calculation)
// -----------------------------------------------------

interface PSIResult {
  psi: number;
  driftDetected: boolean;
  bucketComparison: Array<{
    bucket: string;
    expected: number;
    actual: number;
    contribution: number;
  }>;
}

/**
 * Calculate Population Stability Index between training and production distributions
 * PSI > 0.1 indicates moderate shift, > 0.25 indicates significant drift
 */
export async function calculatePSI(modelVersion: string, windowHours: number = 24): Promise<PSIResult> {
  // Expected distribution from training (stored baseline)
  const expectedDistribution: Record<string, number> = {
    low: 0.7,      // 70% low risk in training
    medium: 0.2,   // 20% medium risk
    high: 0.08,    // 8% high risk
    critical: 0.02 // 2% critical risk
  };
  
  try {
    // Get actual distribution from production
    const result = await query(
      `SELECT 
        risk_bucket, 
        COUNT(*)::float / NULLIF(SUM(COUNT(*)) OVER (), 0) as proportion
       FROM shadow_fraud_scores
       WHERE model_version = $1
         AND scored_at >= NOW() - INTERVAL '1 hour' * $2
       GROUP BY risk_bucket`,
      [modelVersion, windowHours]
    );
    
    const actualDistribution: Record<string, number> = {
      low: 0, medium: 0, high: 0, critical: 0
    };
    
    for (const row of result.rows) {
      actualDistribution[row.risk_bucket as string] = parseFloat(row.proportion as string) || 0;
    }
    
    // Calculate PSI
    let psi = 0;
    const bucketComparison: PSIResult['bucketComparison'] = [];
    
    for (const bucket of ['low', 'medium', 'high', 'critical']) {
      const expected = expectedDistribution[bucket] || 0.001;
      const actual = actualDistribution[bucket] || 0.001;
      const contribution = (actual - expected) * Math.log(actual / expected);
      psi += contribution;
      bucketComparison.push({ bucket, expected, actual, contribution });
    }
    
    const driftDetected = psi > 0.25;
    
    if (driftDetected) {
      void sendAlert({
        severity: 'high',
        component: 'ML_FRAUD',
        title: 'Significant ML Model Drift Detected',
        message: `Fraud model ${modelVersion} has PSI=${psi.toFixed(4)}, which exceeds the critical threshold of 0.25.`,
        metadata: {
          psi,
          modelVersion,
          bucketComparison,
        },
      });
    } else if (psi > 0.1) {
      void sendAlert({
        severity: 'medium',
        component: 'ML_FRAUD',
        title: 'Moderate ML Model Drift Detected',
        message: `Fraud model ${modelVersion} has PSI=${psi.toFixed(4)}, indicating moderate shift (threshold 0.1).`,
        metadata: {
          psi,
          modelVersion,
        },
      });
    }
    
    // Update Prometheus gauges
    shadowFraudPsiGauge.set({ model_version: modelVersion }, psi);
    shadowFraudDriftGauge.set({ model_version: modelVersion }, driftDetected ? 1 : 0);
    
    return { psi, driftDetected, bucketComparison };
  } catch (error) {
    logger.warn('PSI calculation failed', { error, modelVersion });
    return { psi: 0, driftDetected: false, bucketComparison: [] };
  }
}

export default {
  scoreShadow,
  calculatePSI,
  extractFeatures,
  heuristicScore,
};
