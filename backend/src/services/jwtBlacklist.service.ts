/**
 * JWT Blacklist Service
 * 
 * Provides Redis-based token blacklisting for immediate token invalidation on logout.
 * Uses Redis with TTL matching token expiration to automatically clean up.
 * 
 * Design decisions:
 * - Uses Redis SET with TTL to auto-expire entries when tokens would naturally expire
 * - FAIL-CLOSED by default: if Redis is unavailable, tokens are treated as blacklisted
 *   (this is the secure default - prevents use of invalidated tokens during Redis outages)
 * - Key format: jwt:blacklist:{jti} to avoid collisions with other Redis keys
 * - Value is "1" (minimal storage) - we only need to know the key exists
 * - Configurable timeout to prevent auth latency during Redis slowdowns
 */

import { redis } from '../utils/redis';
import logger from '../utils/logger';

const BLACKLIST_KEY_PREFIX = 'jwt:blacklist:';

// Configuration
const BLACKLIST_CHECK_TIMEOUT_MS = parseInt(process.env.JWT_BLACKLIST_TIMEOUT_MS || '500', 10);
// Default to fail-closed (secure) - set to 'false' to allow fail-open (legacy, insecure)
const BLACKLIST_FAIL_CLOSED = process.env.JWT_BLACKLIST_FAIL_CLOSED !== 'false';

/**
 * Add a JWT to the blacklist
 * 
 * @param jti - JWT ID (unique token identifier)
 * @param ttlSeconds - Time-to-live in seconds (should match remaining token validity)
 * @returns true if blacklisted successfully, false if Redis unavailable
 */
export async function blacklistToken(jti: string, ttlSeconds: number): Promise<boolean> {
  if (!jti || ttlSeconds <= 0) {
    return false;
  }

  try {
    const key = `${BLACKLIST_KEY_PREFIX}${jti}`;
    const success = await redis.setEx(key, ttlSeconds, '1');
    
    if (success) {
      logger.info('Token blacklisted', { jti, ttlSeconds });
    }
    
    return success;
  } catch (error) {
    logger.error('Failed to blacklist token', { jti, error });
    return false;
  }
}

/**
 * Check if a JWT is blacklisted
 * 
 * SECURITY: This function uses fail-closed by default.
 * If Redis is unavailable or times out, the token is treated as blacklisted
 * to prevent use of invalidated tokens during Redis outages.
 * 
 * @param jti - JWT ID to check
 * @returns true if blacklisted OR if Redis unavailable (fail-closed for security)
 */
export async function isTokenBlacklisted(jti: string): Promise<boolean> {
  if (!jti) {
    return false;
  }

  try {
    const key = `${BLACKLIST_KEY_PREFIX}${jti}`;
    
    // Add timeout to prevent auth latency during Redis slowdowns
    const value = await Promise.race([
      redis.get<string>(key),
      new Promise<null>((_, reject) =>
        setTimeout(() => reject(new Error('Blacklist check timeout')), BLACKLIST_CHECK_TIMEOUT_MS)
      ),
    ]);
    
    return value !== null;
  } catch (error) {
    // Log the failure for monitoring
    logger.error('Failed to check token blacklist', { 
      jti, 
      error: error instanceof Error ? error.message : String(error),
      failClosed: BLACKLIST_FAIL_CLOSED,
    });
    
    if (BLACKLIST_FAIL_CLOSED) {
      // FAIL-CLOSED (secure default): Treat as blacklisted when we can't verify
      // This prevents use of potentially invalidated tokens during Redis outages
      logger.warn('Treating token as blacklisted due to Redis failure (fail-closed)', { jti });
      return true;
    }
    
    // Legacy fail-open mode (should NOT be used in production)
    // Only enable if there's a specific operational reason and you understand the security risk
    logger.warn('INSECURE: Allowing token due to blacklist check failure (fail-open mode)', { jti });
    return false;
  }
}

/**
 * Calculate remaining TTL for a token based on its expiration timestamp
 * 
 * @param exp - Token expiration timestamp (Unix seconds)
 * @returns Remaining seconds until expiration, or 0 if already expired
 */
export function calculateRemainingTtl(exp: number): number {
  const now = Math.floor(Date.now() / 1000);
  const remaining = exp - now;
  return Math.max(0, remaining);
}

export default {
  blacklistToken,
  isTokenBlacklisted,
  calculateRemainingTtl,
};
