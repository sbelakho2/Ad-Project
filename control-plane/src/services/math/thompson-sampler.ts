/**
 * THOMPSON SAMPLER (The Optimizer)
 * 
 * Zero-cost multi-armed bandit for autonomous optimization.
 * Uses Beta-Bernoulli model for subject lines, templates, and send times.
 * 
 * Key insight: This converges MUCH faster than A/B testing for low-volume
 * cold email because it immediately starts exploiting winners while still
 * exploring alternatives.
 * 
 * Mathematical Background:
 * - Each "arm" (subject line) has a Beta(α, β) distribution
 * - α = 1 + successes (opens/replies)
 * - β = 1 + failures (sent - successes)
 * - We sample from each distribution and pick the highest
 * - This naturally balances exploration vs exploitation
 */

import { queryReadOnly, queryOperator } from '../../database';
import { logger } from '../../logger';

// ============================================================================
// TYPES
// ============================================================================

export interface BanditArm {
  id: string;
  armType: string;
  armName: string;
  armValue: string;
  alpha: number;
  beta: number;
  totalTrials: number;
  totalSuccesses: number;
  empiricalRate: number;
  lastSampledScore: number | null;
  active: boolean;
}

export interface SamplingResult {
  armId: string;
  armValue: string;
  sampledScore: number;
  competingArmsCount: number;
}

// ============================================================================
// BETA DISTRIBUTION SAMPLING (Pure Math - Zero Cost)
// ============================================================================

/**
 * Sample from a Beta distribution using the Gamma function approximation.
 * This is the core of Thompson Sampling.
 * 
 * Uses the fact that if X ~ Gamma(α, 1) and Y ~ Gamma(β, 1),
 * then X / (X + Y) ~ Beta(α, β)
 */
function sampleBeta(alpha: number, beta: number): number {
  // Generate two Gamma random variables
  const x = sampleGamma(alpha);
  const y = sampleGamma(beta);
  
  // Return Beta sample
  return x / (x + y);
}

/**
 * Sample from a Gamma distribution using Marsaglia and Tsang's method.
 * Works for shape >= 1. For shape < 1, we use transformation.
 */
function sampleGamma(shape: number): number {
  if (shape < 1) {
    // For shape < 1, use the transformation:
    // If X ~ Gamma(shape+1), then X * U^(1/shape) ~ Gamma(shape)
    // where U ~ Uniform(0, 1)
    return sampleGamma(shape + 1) * Math.pow(Math.random(), 1 / shape);
  }
  
  // Marsaglia and Tsang's method for shape >= 1
  const d = shape - 1 / 3;
  const c = 1 / Math.sqrt(9 * d);
  
  while (true) {
    let x: number;
    let v: number;
    
    do {
      x = randomNormal();
      v = 1 + c * x;
    } while (v <= 0);
    
    v = v * v * v;
    const u = Math.random();
    
    // Accept/reject
    if (u < 1 - 0.0331 * (x * x) * (x * x)) {
      return d * v;
    }
    
    if (Math.log(u) < 0.5 * x * x + d * (1 - v + Math.log(v))) {
      return d * v;
    }
  }
}

/**
 * Generate standard normal random variable using Box-Muller transform.
 */
function randomNormal(): number {
  const u1 = Math.random();
  const u2 = Math.random();
  return Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
}

// ============================================================================
// CORE THOMPSON SAMPLER CLASS
// ============================================================================

export class ThompsonSampler {
  private arms: Map<string, { alpha: number; beta: number }> = new Map();
  
  /**
   * Add an arm to the bandit
   */
  addOption(armId: string, priorAlpha = 1, priorBeta = 1): void {
    this.arms.set(armId, { alpha: priorAlpha, beta: priorBeta });
  }
  
  /**
   * Choose the best arm using Thompson Sampling
   */
  choose(): string {
    if (this.arms.size === 0) {
      throw new Error('No arms available');
    }
    
    let bestArm = '';
    let bestScore = -Infinity;
    
    for (const [armId, params] of this.arms) {
      const score = sampleBeta(params.alpha, params.beta);
      if (score > bestScore) {
        bestScore = score;
        bestArm = armId;
      }
    }
    
    return bestArm;
  }
  
  /**
   * Update arm after observing outcome
   */
  update(armId: string, success: boolean): void {
    const arm = this.arms.get(armId);
    if (!arm) return;
    
    if (success) {
      arm.alpha += 1;
    } else {
      arm.beta += 1;
    }
  }
  
  /**
   * Get the current empirical rate for an arm
   */
  getRate(armId: string): number {
    const arm = this.arms.get(armId);
    if (!arm) return 0;
    
    const total = arm.alpha + arm.beta - 2; // Subtract priors
    if (total === 0) return 0;
    
    return (arm.alpha - 1) / total;
  }
  
  /**
   * Get all arms with their stats
   */
  getStats(): Array<{ armId: string; alpha: number; beta: number; rate: number }> {
    return Array.from(this.arms.entries()).map(([armId, params]) => ({
      armId,
      alpha: params.alpha,
      beta: params.beta,
      rate: this.getRate(armId),
    }));
  }
}

// ============================================================================
// DATABASE-BACKED OPERATIONS
// ============================================================================

/**
 * Get all active arms for a given type
 */
export async function getActiveArms(armType: string): Promise<BanditArm[]> {
  const rows = await queryReadOnly<{
    id: string;
    arm_type: string;
    arm_name: string;
    arm_value: string;
    alpha: number;
    beta: number;
    total_trials: number;
    total_successes: number;
    empirical_rate: string;
    last_sampled_score: string | null;
    active: boolean;
  }>(`
    SELECT id, arm_type, arm_name, arm_value, alpha, beta, 
           total_trials, total_successes, empirical_rate,
           last_sampled_score, active
    FROM sa_bandit_arms
    WHERE arm_type = $1 AND active = true
    ORDER BY total_trials DESC
  `, [armType]);
  
  return rows.map(row => ({
    id: row.id,
    armType: row.arm_type,
    armName: row.arm_name,
    armValue: row.arm_value,
    alpha: row.alpha,
    beta: row.beta,
    totalTrials: row.total_trials,
    totalSuccesses: row.total_successes,
    empiricalRate: parseFloat(row.empirical_rate || '0'),
    lastSampledScore: row.last_sampled_score ? parseFloat(row.last_sampled_score) : null,
    active: row.active,
  }));
}

/**
 * Sample from the bandit and select the best arm
 */
export async function sampleAndSelect(armType: string): Promise<SamplingResult | null> {
  const arms = await getActiveArms(armType);
  
  if (arms.length === 0) {
    logger.warn('No active bandit arms for type', { armType });
    return null;
  }
  
  // If only one arm, just return it
  if (arms.length === 1) {
    return {
      armId: arms[0].id,
      armValue: arms[0].armValue,
      sampledScore: 1.0,
      competingArmsCount: 1,
    };
  }
  
  // Sample from each arm's Beta distribution
  let bestArm: BanditArm | null = null;
  let bestScore = -Infinity;
  
  for (const arm of arms) {
    const score = sampleBeta(arm.alpha, arm.beta);
    
    // Update the sampled score in memory (will persist later)
    arm.lastSampledScore = score;
    
    if (score > bestScore) {
      bestScore = score;
      bestArm = arm;
    }
  }
  
  if (!bestArm) {
    return null;
  }
  
  // Record the selection
  await queryOperator(`
    INSERT INTO sa_bandit_trials (arm_id, sampled_score, competing_arms_count)
    VALUES ($1, $2, $3)
  `, [bestArm.id, bestScore, arms.length]);
  
  // Update the arm's last sampled score
  await queryOperator(`
    UPDATE sa_bandit_arms 
    SET last_sampled_score = $1, last_sampled_at = CURRENT_TIMESTAMP
    WHERE id = $2
  `, [bestScore, bestArm.id]);
  
  logger.info('Thompson sampling selected arm', {
    armType,
    selectedArm: bestArm.armName,
    sampledScore: bestScore.toFixed(4),
    competingArms: arms.length,
  });
  
  return {
    armId: bestArm.id,
    armValue: bestArm.armValue,
    sampledScore: bestScore,
    competingArmsCount: arms.length,
  };
}

/**
 * Record outcome for a trial
 */
export async function recordOutcome(
  armId: string,
  success: boolean,
  trialId?: string
): Promise<void> {
  // Update the arm's alpha/beta
  if (success) {
    await queryOperator(`
      UPDATE sa_bandit_arms
      SET alpha = alpha + 1,
          total_trials = total_trials + 1,
          total_successes = total_successes + 1,
          updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [armId]);
  } else {
    await queryOperator(`
      UPDATE sa_bandit_arms
      SET beta = beta + 1,
          total_trials = total_trials + 1,
          updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [armId]);
  }
  
  // Update the trial record if provided
  if (trialId) {
    await queryOperator(`
      UPDATE sa_bandit_trials
      SET success = $1, outcome_recorded_at = CURRENT_TIMESTAMP
      WHERE id = $2
    `, [success, trialId]);
  }
  
  logger.debug('Recorded bandit outcome', { armId, success });
}

/**
 * Create a new bandit arm
 */
export async function createArm(params: {
  armType: string;
  armName: string;
  armValue: string;
  priorAlpha?: number;
  priorBeta?: number;
}): Promise<string> {
  const { armType, armName, armValue, priorAlpha = 1, priorBeta = 1 } = params;
  
  const result = await queryOperator<{ id: string }>(`
    INSERT INTO sa_bandit_arms (arm_type, arm_name, arm_value, alpha, beta)
    VALUES ($1, $2, $3, $4, $5)
    RETURNING id
  `, [armType, armName, armValue, priorAlpha, priorBeta]);
  
  logger.info('Created new bandit arm', { armType, armName });
  
  return result[0].id;
}

/**
 * Retire an arm that is underperforming
 */
export async function retireArm(armId: string, reason: string): Promise<void> {
  await queryOperator(`
    UPDATE sa_bandit_arms
    SET active = false, retired_at = CURRENT_TIMESTAMP, retirement_reason = $2
    WHERE id = $1
  `, [armId, reason]);
  
  logger.info('Retired bandit arm', { armId, reason });
}

/**
 * Get bandit statistics summary
 */
export async function getBanditStats(armType: string): Promise<{
  totalArms: number;
  activeArms: number;
  totalTrials: number;
  bestPerformer: { name: string; rate: number } | null;
  convergenceScore: number;
}> {
  const arms = await getActiveArms(armType);
  
  if (arms.length === 0) {
    return {
      totalArms: 0,
      activeArms: 0,
      totalTrials: 0,
      bestPerformer: null,
      convergenceScore: 0,
    };
  }
  
  const totalTrials = arms.reduce((sum, arm) => sum + arm.totalTrials, 0);
  const sortedByRate = [...arms].sort((a, b) => b.empiricalRate - a.empiricalRate);
  const best = sortedByRate[0];
  
  // Convergence score: How confident are we that we've found the winner?
  // Based on the gap between best and second-best performance
  let convergenceScore = 0;
  if (sortedByRate.length >= 2 && totalTrials > 20) {
    const gap = best.empiricalRate - sortedByRate[1].empiricalRate;
    convergenceScore = Math.min(100, Math.round(gap * 500)); // 20% gap = 100% converged
  }
  
  return {
    totalArms: arms.length,
    activeArms: arms.filter(a => a.active).length,
    totalTrials,
    bestPerformer: best ? { name: best.armName, rate: best.empiricalRate } : null,
    convergenceScore,
  };
}

// Export for testing
export { sampleBeta, sampleGamma, randomNormal };
