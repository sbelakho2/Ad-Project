/**
 * Auction Idempotency Service
 * 
 * Provides request_id-based deduplication for auction requests.
 * Uses Redis with short TTL to return identical responses for duplicate
 * requests within the idempotency window (helps with client retries/hedging).
 */

import redis from '../../utils/redis';
import logger from '../../utils/logger';
import crypto from 'crypto';

/** TTL for cached auction responses (seconds) */
const IDEMPOTENCY_TTL_SEC = parseInt(process.env.AUCTION_IDEMPOTENCY_TTL_SEC || '30', 10);

/** Key prefix for auction idempotency cache */
const KEY_PREFIX = 'auction:idem:';

export interface CachedAuctionResult {
  landscapeId: string;
  success: boolean;
  response?: {
    requestId: string;
    bidId: string;
    adapter: string;
    cpm: number;
    currency: string;
    ttlSeconds: number;
    creativeUrl: string;
    tracking: { impression: string; click: string };
    payload: Record<string, unknown>;
    consentEcho?: Record<string, unknown>;
  };
  reason?: string;
  cachedAt: number;
}

/**
 * Generate a deterministic landscape_id from the auction result
 * This provides a stable identifier for bid landscape export
 * 
 * IMPORTANT: This function must be deterministic - same inputs always produce
 * same output. This is required for idempotency (retries return same landscape ID).
 */
export function generateLandscapeId(requestId: string, bidId?: string): string {
  const hash = crypto.createHash('sha256')
    .update(`${requestId}:${bidId || 'no-bid'}`)
    .digest('hex')
    .substring(0, 24);
  return `ls_${hash}`;
}

/**
 * Check if a cached response exists for this request_id
 */
export async function getCachedAuctionResult(requestId: string): Promise<CachedAuctionResult | null> {
  if (!requestId || !redis.isReady()) {
    return null;
  }

  try {
    const key = `${KEY_PREFIX}${requestId}`;
    const cached = await redis.get<CachedAuctionResult>(key);
    if (cached) {
      logger.debug('Auction idempotency cache hit', { requestId, landscapeId: cached.landscapeId });
    }
    return cached;
  } catch (error) {
    logger.warn('Auction idempotency cache get failed', { error, requestId });
    return null;
  }
}

/**
 * Store auction result in idempotency cache
 */
export async function cacheAuctionResult(
  requestId: string,
  result: Omit<CachedAuctionResult, 'cachedAt'>
): Promise<boolean> {
  if (!requestId || !redis.isReady()) {
    return false;
  }

  try {
    const key = `${KEY_PREFIX}${requestId}`;
    const toStore: CachedAuctionResult = {
      ...result,
      cachedAt: Date.now(),
    };
    const success = await redis.set(key, toStore, IDEMPOTENCY_TTL_SEC);
    if (success) {
      logger.debug('Auction result cached', { requestId, landscapeId: result.landscapeId, ttl: IDEMPOTENCY_TTL_SEC });
    }
    return success;
  } catch (error) {
    logger.warn('Auction idempotency cache set failed', { error, requestId });
    return false;
  }
}

/**
 * Check if Redis is available for idempotency checks
 */
export function isIdempotencyEnabled(): boolean {
  return redis.isReady() && IDEMPOTENCY_TTL_SEC > 0;
}

export default {
  getCachedAuctionResult,
  cacheAuctionResult,
  generateLandscapeId,
  isIdempotencyEnabled,
};
