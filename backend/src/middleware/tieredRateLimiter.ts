/**
 * Tiered Rate Limiter Middleware
 * 
 * Implements tier-aware rate limiting with Redis backing.
 * Uses simple increment/expire pattern compatible with RedisClient wrapper.
 * Falls back to in-memory limiting when Redis is unavailable.
 */

import { Request, Response, NextFunction } from 'express';
import rateLimit from 'express-rate-limit';
import { redis } from '../utils/redis';
import { logger } from '../utils/logger';
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
 */
function generateKey(req: Request, category: keyof TierRateLimits): string {
  const userReq = req as RequestWithUser;
  const publisherId = userReq.user?.publisherId || 'anon';
  const ip = getClientIp(req);
  const tier = userReq.user?.tier || 'unauthenticated';
  
  // Key format: rl:{category}:{tier}:{publisherId}:{ip}
  return `rl:${category}:${tier}:${publisherId}:${ip}`;
}

/**
 * Get rate limits for the current request based on user tier
 */
function getLimitsForRequest(req: Request, category: keyof TierRateLimits): RateLimitConfig {
  const userReq = req as RequestWithUser;
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
 * Create a rate limiter middleware for a specific category
 */
export function createTieredRateLimiter(
  category: keyof TierRateLimits,
  options: { burstMultiplier?: number } = {}
): (req: Request, res: Response, next: NextFunction) => void {
  const { burstMultiplier = 1 } = options;

  return async (req: Request, res: Response, next: NextFunction) => {
    // Skip exempt routes
    if (isRateLimitExempt(req.originalUrl || req.path)) {
      return next();
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
 */
export const analyticsRateLimiter = createTieredRateLimiter('analytics');

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
