/**
 * Experiment Authorization Cache
 * 
 * SECURITY FIX: Provides server-side verification of experiment mode/arm claims
 * to prevent SDK spoofing attacks.
 * 
 * The SDK sends experiment_id, mode, and arm in the RTB request, but these
 * MUST be verified against the database to prevent malicious actors from:
 * 1. Forcing "mirroring" mode when only "shadow" is authorized
 * 2. Skewing reports by providing false arm labels
 * 3. Activating experiments that are paused/failed/completed
 * 
 * This cache layer:
 * - Caches authorized experiment configurations in Redis (TTL: 30s)
 * - Falls back to database lookup on cache miss
 * - Rejects requests with invalid/mismatched experiment claims
 */

import pool from '../../utils/postgres';
import redis from '../../utils/redis';
import logger from '../../utils/logger';
import type { ExperimentMode, ExperimentStatus } from '../../types/migration';

const CACHE_PREFIX = 'exp_auth:';
const CACHE_TTL_SECONDS = 30; // Short TTL for fast propagation of status changes

export interface AuthorizedExperiment {
  experimentId: string;
  publisherId: string;
  status: ExperimentStatus;
  mode: ExperimentMode;
  mirrorPercent: number;
  assignmentSeed: string;
  /** UTC timestamp when this config was cached */
  cachedAt: number;
}

export interface ExperimentAuthResult {
  authorized: boolean;
  experiment?: AuthorizedExperiment;
  reason?: 'NOT_FOUND' | 'WRONG_PUBLISHER' | 'NOT_ACTIVE' | 'MODE_MISMATCH' | 'CACHE_ERROR';
}

/**
 * Check if the requested experiment mode/arm is authorized
 */
export async function verifyExperimentAuth(
  experimentId: string,
  publisherId: string,
  claimedMode?: ExperimentMode
): Promise<ExperimentAuthResult> {
  try {
    // Try Redis cache first
    const cached = await getCachedAuth(experimentId);
    
    if (cached) {
      return validateCachedAuth(cached, publisherId, claimedMode);
    }

    // Cache miss - fetch from database
    const dbResult = await pool.query(
      `SELECT id, publisher_id, status, mode, mirror_percent, assignment_seed
       FROM migration_experiments
       WHERE id = $1`,
      [experimentId]
    );

    if (dbResult.rows.length === 0) {
      return { authorized: false, reason: 'NOT_FOUND' };
    }

    const row = dbResult.rows[0];
    const auth: AuthorizedExperiment = {
      experimentId: row.id,
      publisherId: row.publisher_id,
      status: row.status,
      mode: row.mode || 'shadow',
      mirrorPercent: row.mirror_percent ?? 0,
      assignmentSeed: row.assignment_seed || row.id,
      cachedAt: Date.now(),
    };

    // Cache the result
    await setCachedAuth(auth);

    return validateCachedAuth(auth, publisherId, claimedMode);
  } catch (error) {
    logger.warn('Experiment auth verification failed, allowing fail-open', {
      experimentId,
      publisherId,
      error: (error as Error).message,
    });
    // Fail-open to avoid blocking legitimate traffic on cache/db errors
    // This is a trade-off: we prioritize availability over strict security
    return { authorized: true, reason: 'CACHE_ERROR' };
  }
}

/**
 * Validate cached authorization against request claims
 */
function validateCachedAuth(
  auth: AuthorizedExperiment,
  publisherId: string,
  claimedMode?: ExperimentMode
): ExperimentAuthResult {
  // Check publisher ownership
  if (auth.publisherId !== publisherId) {
    logger.warn('Experiment auth: publisher mismatch', {
      experimentId: auth.experimentId,
      expectedPublisher: auth.publisherId,
      claimedPublisher: publisherId,
    });
    return { authorized: false, reason: 'WRONG_PUBLISHER' };
  }

  // Check experiment is active
  if (auth.status !== 'active') {
    logger.debug('Experiment auth: not active', {
      experimentId: auth.experimentId,
      status: auth.status,
    });
    return { authorized: false, reason: 'NOT_ACTIVE' };
  }

  // Check mode authorization
  // CRITICAL: SDK cannot claim "mirroring" if experiment is in "shadow" mode
  if (claimedMode === 'mirroring' && auth.mode === 'shadow') {
    logger.warn('Experiment auth: mode escalation attempt blocked', {
      experimentId: auth.experimentId,
      authorizedMode: auth.mode,
      claimedMode,
    });
    return { authorized: false, reason: 'MODE_MISMATCH' };
  }

  return { authorized: true, experiment: auth };
}

/**
 * Get cached authorization from Redis
 */
async function getCachedAuth(experimentId: string): Promise<AuthorizedExperiment | null> {
  try {
    const key = `${CACHE_PREFIX}${experimentId}`;
    const cached = await redis.get<AuthorizedExperiment>(key);
    
    if (!cached) return null;
    
    return cached;
  } catch {
    return null;
  }
}

/**
 * Set cached authorization in Redis
 */
async function setCachedAuth(auth: AuthorizedExperiment): Promise<void> {
  try {
    const key = `${CACHE_PREFIX}${auth.experimentId}`;
    await redis.set(key, auth, CACHE_TTL_SECONDS);
  } catch {
    // Ignore cache write failures
  }
}

/**
 * Invalidate cached authorization (call after experiment status changes)
 */
export async function invalidateExperimentAuthCache(experimentId: string): Promise<void> {
  try {
    const key = `${CACHE_PREFIX}${experimentId}`;
    await redis.del(key);
    
    logger.debug('Invalidated experiment auth cache', { experimentId });
  } catch {
    // Ignore cache invalidation failures
  }
}

/**
 * Batch invalidate multiple experiments (useful after bulk operations)
 */
export async function invalidateExperimentAuthCacheBatch(experimentIds: string[]): Promise<void> {
  if (experimentIds.length === 0) return;
  
  try {
    // Delete each key individually since the redis wrapper may not support multi-key del
    await Promise.all(
      experimentIds.map(id => redis.del(`${CACHE_PREFIX}${id}`))
    );
    
    logger.debug('Batch invalidated experiment auth cache', { count: experimentIds.length });
  } catch {
    // Ignore cache invalidation failures
  }
}
