/**
 * Tiered Rate Limiter Middleware
 * 
 * Implements tier-aware rate limiting with Redis backing.
 * Uses simple increment/expire pattern compatible with RedisClient wrapper.
 * Falls back to in-memory limiting when Redis is unavailable.
 * 
 * SDK-aware: For analytics endpoints, performs a quick lookup of SDK key
 * to identify publisher and use their tier for rate limiting.
 */

import { Request, Response, NextFunction } from 'express';
import rateLimit from 'express-rate-limit';
import { redis } from '../utils/redis';
import { logger } from '../utils/logger';
import { sha256Hex } from '../utils/crypto';
import {
  TIER_RATE_LIMITS,
  UNAUTHENTICATED_RATE_LIMITS,
  isRateLimitExempt,
  RATE_LIMIT_EXEMPT_ROUTES,
  type TierRateLimits,
  type RateLimitConfig,
} from '../config/rateLimits';
import type { PlatformTierId } from '../config/platformTiers';
import type { AuthTokenPayload } from '../types/auth';

// Use the same RequestWithUser interface as auth middleware
interface RequestWithUser extends Request {
  user?: AuthTokenPayload & {
    tier?: PlatformTierId;
  };
  sdk?: {
    publisherId: string;
    appId: string;
    environment: string;
    keyId: string;
  };
  sdkPreAuth?: {
    publisherId: string;
    tier: PlatformTierId;
  };
}

/**
 * Get client IP address from request
 */
function getClientIp(req: Request): string {
  // Check common proxy headers
  const xForwardedFor = req.headers['x-forwarded-for'];
  if (xForwardedFor) {
    const forwarded = Array.isArray(xForwardedFor) 
      ? xForwardedFor[0] 
      : xForwardedFor.split(',')[0];
    return forwarded.trim();
  }
  
  const xRealIp = req.headers['x-real-ip'];
  if (xRealIp) {
    return Array.isArray(xRealIp) ? xRealIp[0] : xRealIp;
  }
  
  return req.ip || req.socket?.remoteAddress || 'unknown';
}

/**
 * Generate rate limit key based on context
 * SDK-aware: checks sdkPreAuth for pre-authenticated SDK requests
 */
function generateKey(req: Request, category: keyof TierRateLimits): string {
  const userReq = req as RequestWithUser;
  const ip = getClientIp(req);
  
  // Check SDK pre-auth first (set by quick SDK key lookup)
  if (userReq.sdkPreAuth?.publisherId) {
    const tier = userReq.sdkPreAuth.tier || 'starter';
    return `rl:${category}:${tier}:${userReq.sdkPreAuth.publisherId}:${ip}`;
  }
  
  // Then check user auth
  const publisherId = userReq.user?.publisherId || 'anon';
  const tier = userReq.user?.tier || 'unauthenticated';
  
  // Key format: rl:{category}:{tier}:{publisherId}:{ip}
  return `rl:${category}:${tier}:${publisherId}:${ip}`;
}

/**
 * Get rate limits for the current request based on user tier
 * SDK-aware: checks sdkPreAuth for pre-authenticated SDK requests
 */
function getLimitsForRequest(req: Request, category: keyof TierRateLimits): RateLimitConfig {
  const userReq = req as RequestWithUser;
  
  // Check SDK pre-auth first (set by quick SDK key lookup)
  if (userReq.sdkPreAuth?.tier && TIER_RATE_LIMITS[userReq.sdkPreAuth.tier]) {
    return TIER_RATE_LIMITS[userReq.sdkPreAuth.tier][category];
  }
  
  // Then check user auth
  const tier = userReq.user?.tier;
  
  if (tier && TIER_RATE_LIMITS[tier]) {
    return TIER_RATE_LIMITS[tier][category];
  }
  
  // Use unauthenticated limits for anonymous requests
  return UNAUTHENTICATED_RATE_LIMITS[category];
}

/**
 * Check rate limit using Redis with simple increment/expire
 * Returns { allowed: boolean, current: number, limit: number, resetInSeconds: number }
 */
async function checkRateLimit(
  key: string,
  limit: number,
  windowMs: number
): Promise<{ allowed: boolean; current: number; limit: number; resetInSeconds: number }> {
  if (!redis.isReady()) {
    // Fail open - allow request if Redis is unavailable
    return { allowed: true, current: 0, limit, resetInSeconds: 0 };
  }

  try {
    const windowSeconds = Math.ceil(windowMs / 1000);
    
    // Increment counter
    const current = await redis.incr(key);
    
    // Set expiry only on first hit (when counter is 1)
    if (current === 1) {
      await redis.expire(key, windowSeconds);
    }
    
    // Get TTL for reset header
    const ttl = await redis.ttl(key);
    const resetInSeconds = Math.max(1, ttl || windowSeconds);
    
    return {
      allowed: current <= limit,
      current,
      limit,
      resetInSeconds,
    };
  } catch (error) {
    logger.error('Rate limit check failed', { error, key });
    // Fail open on errors
    return { allowed: true, current: 0, limit, resetInSeconds: 0 };
  }
}

/**
 * Quick SDK key lookup for rate limiting purposes.
 * Does a fast Redis cache check, then falls back to DB lookup.
 * Caches publisher ID and tier for 5 minutes to avoid repeated lookups.
 */
async function lookupSdkKeyForRateLimit(req: Request): Promise<{ publisherId: string; tier: PlatformTierId } | null> {
  // Extract SDK key from headers
  const sdkKeyHeader = (req.header('x-sdk-key') || req.header('X-Sdk-Key') || '').trim();
  const apiKeyHeader = (req.header('x-api-key') || req.header('X-Api-Key') || '').trim();
  const authHeader = (req.header('authorization') || '').trim();
  
  let key = '';
  if (authHeader.toLowerCase().startsWith('bearer ')) {
    const token = authHeader.slice(7).trim();
    // Skip JWT tokens - they're for user auth, not SDK auth
    if (!token.startsWith('eyJ')) {
      key = token;
    }
  }
  if (!key) key = apiKeyHeader;
  if (!key) key = sdkKeyHeader;
  
  // No SDK key provided
  if (!key || !key.startsWith('ak_')) {
    return null;
  }

  const digest = sha256Hex(key);
  const cacheKey = `sdk_rl:${digest}`;
  
  try {
    // Check Redis cache first
    if (redis.isReady()) {
      const cached = await redis.get<{ p: string; t: string }>(cacheKey);
      if (cached) {
        return { publisherId: cached.p, tier: cached.t as PlatformTierId };
      }
    }
    
    // Cache miss - do lightweight DB lookup (no bcrypt verify, just digest match)
    // Import inline to avoid circular dependency
    const { query } = await import('../utils/postgres');
    const result = await query<{ publisher_id: string; revoked_at: string | null }>(
      `SELECT publisher_id, revoked_at FROM app_api_keys WHERE secret_digest = $1 LIMIT 1`,
      [digest]
    );
    
    if (!result.rows[0] || result.rows[0].revoked_at) {
      return null;
    }
    
    const publisherId = result.rows[0].publisher_id;
    
    // Look up publisher's tier via their subscription (default to starter if not found)
    // Join through users table: publisher -> users -> subscriptions
    const tierResult = await query<{ plan_type: string }>(
      `SELECT s.plan_type 
         FROM subscriptions s
         JOIN users u ON u.id = s.customer_id
        WHERE u.publisher_id = $1 
          AND s.status = 'active'
        ORDER BY s.created_at DESC
        LIMIT 1`,
      [publisherId]
    );
    const tier = (tierResult.rows[0]?.plan_type as PlatformTierId) || 'starter';
    
    // Cache for 5 minutes
    if (redis.isReady()) {
      await redis.set(cacheKey, { p: publisherId, t: tier }, 300);
    }
    
    return { publisherId, tier };
  } catch (error) {
    logger.debug('SDK key rate limit lookup failed', { error });
    return null;
  }
}

/**
 * Create a rate limiter middleware for a specific category
 */
export function createTieredRateLimiter(
  category: keyof TierRateLimits,
  options: { burstMultiplier?: number; sdkAware?: boolean } = {}
): (req: Request, res: Response, next: NextFunction) => void {
  const { burstMultiplier = 1, sdkAware = false } = options;

  return async (req: Request, res: Response, next: NextFunction) => {
    // Skip exempt routes
    if (isRateLimitExempt(req.originalUrl || req.path)) {
      return next();
    }
    
    // For SDK-aware rate limiters (analytics ingestion), do quick SDK key lookup
    if (sdkAware) {
      const sdkInfo = await lookupSdkKeyForRateLimit(req);
      if (sdkInfo) {
        (req as RequestWithUser).sdkPreAuth = sdkInfo;
      }
    }
    const limits = getLimitsForRequest(req, category);
    const effectiveLimit = Math.floor(limits.max * burstMultiplier);
    const key = generateKey(req, category);

    const result = await checkRateLimit(key, effectiveLimit, limits.windowMs);

    // Set standard rate limit headers
    res.setHeader('X-RateLimit-Limit', String(result.limit));
    res.setHeader('X-RateLimit-Remaining', String(Math.max(0, result.limit - result.current)));
    res.setHeader('X-RateLimit-Reset', String(Math.floor(Date.now() / 1000) + result.resetInSeconds));

    if (!result.allowed) {
      const userReq = req as RequestWithUser;
      res.setHeader('Retry-After', String(result.resetInSeconds));
      
      logger.warn('Rate limit exceeded', {
        category,
        tier: userReq.user?.tier || 'unauthenticated',
        publisherId: userReq.user?.publisherId || 'anon',
        ip: getClientIp(req),
        current: result.current,
        limit: result.limit,
        path: req.path,
      });

      return res.status(429).json({
        success: false,
        error: 'Too many requests',
        message: 'Rate limit exceeded. Please try again later or upgrade your plan for higher limits.',
        retryAfter: result.resetInSeconds,
        upgradeUrl: '/pricing',
      });
    }

    return next();
  };
}

/**
 * Auth-specific rate limiter with stricter limits
 */
export const authTieredRateLimiter = createTieredRateLimiter('auth');

/**
 * Dashboard API rate limiter
 */
export const dashboardRateLimiter = createTieredRateLimiter('dashboard');

/**
 * Analytics/SDK ingest rate limiter with higher limits
 * SDK-aware: performs quick SDK key lookup to use publisher's tier limits
 */
export const analyticsRateLimiter = createTieredRateLimiter('analytics', { sdkAware: true });

/**
 * Transparency/VRA API rate limiter
 */
export const transparencyRateLimiter = createTieredRateLimiter('transparency');

/**
 * RTB/bidding API rate limiter with burst support
 */
export const rtbRateLimiter = createTieredRateLimiter('rtb', { burstMultiplier: 2 });

/**
 * Global rate limiter for general API endpoints
 */
export const globalRateLimiter = createTieredRateLimiter('global');

/**
 * Default fallback rate limiter using express-rate-limit (in-memory)
 * Used when Redis is unavailable or for simple limiting
 */
export const defaultRateLimiter = rateLimit({
  windowMs: parseInt(process.env.RATE_LIMIT_WINDOW_MS || '60000', 10), // 1 minute default
  max: parseInt(process.env.RATE_LIMIT_MAX_REQUESTS || '1000', 10),    // 1000/min default
  standardHeaders: true,
  legacyHeaders: false,
  message: {
    success: false,
    error: 'Too many requests',
    message: 'Rate limit exceeded. Please try again later.',
  },
  skip: (req: Request) => isRateLimitExempt(req.originalUrl || req.path),
});

// Re-export isRateLimitExempt for use in index.ts
export { isRateLimitExempt, RATE_LIMIT_EXEMPT_ROUTES };
