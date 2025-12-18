/**
 * Tiered Rate Limiting Configuration
 * 
 * Industry benchmarks (as of Dec 2025):
 * - Stripe: 100 req/sec live, endpoint-specific limits of 25 req/sec
 * - Twitter/X Pro: 900 req/15min (60/min), Enterprise: custom
 * - GitHub: 5,000 req/hour authenticated (83/min), 60/hour unauthenticated
 * - AWS API Gateway: 10,000 req/sec burst, 5,000 sustained
 * 
 * ApexMediation rate limits are designed for:
 * - SDK telemetry (high volume, bursty)
 * - Dashboard API (moderate volume, interactive)
 * - Auth endpoints (low volume, security-sensitive)
 * - Public endpoints (cacheable, exempt from strict limits)
 */

import type { PlatformTierId } from './platformTiers';

export interface RateLimitConfig {
  /** Requests allowed per window */
  max: number;
  /** Window duration in milliseconds */
  windowMs: number;
  /** Human-readable description */
  description: string;
}

export interface TierRateLimits {
  /** Global API rate limit (all endpoints except exempted) */
  global: RateLimitConfig;
  /** Auth endpoints (login, register, password reset) */
  auth: RateLimitConfig;
  /** Dashboard/Console API calls */
  dashboard: RateLimitConfig;
  /** Analytics ingestion (SDK events) */
  analytics: RateLimitConfig;
  /** Transparency/VRA API */
  transparency: RateLimitConfig;
  /** RTB/SSAI endpoints */
  rtb: RateLimitConfig;
}

/**
 * Rate limits per customer tier
 * 
 * Tiers scale roughly 5x between each level:
 * - Starter: Solo developers, small apps (<1M impressions/mo)
 * - Growth: Growing studios (1-10M impressions/mo)
 * - Scale: Large publishers (10-50M impressions/mo)
 * - Enterprise: High-volume, custom needs (50M+ impressions/mo)
 */
export const TIER_RATE_LIMITS: Record<PlatformTierId, TierRateLimits> = {
  starter: {
    global: {
      max: 1000,
      windowMs: 60_000, // 1 minute
      description: '1,000 requests/minute global',
    },
    auth: {
      max: 20,
      windowMs: 60_000,
      description: '20 auth requests/minute (brute-force protection)',
    },
    dashboard: {
      max: 300,
      windowMs: 60_000,
      description: '300 dashboard requests/minute',
    },
    analytics: {
      max: 5_000,
      windowMs: 60_000,
      description: '5,000 analytics events/minute',
    },
    transparency: {
      max: 120,
      windowMs: 60_000,
      description: '120 transparency API requests/minute',
    },
    rtb: {
      max: 1_000,
      windowMs: 60_000,
      description: '1,000 RTB/SSAI requests/minute',
    },
  },

  growth: {
    global: {
      max: 5_000,
      windowMs: 60_000,
      description: '5,000 requests/minute global',
    },
    auth: {
      max: 50,
      windowMs: 60_000,
      description: '50 auth requests/minute',
    },
    dashboard: {
      max: 1_000,
      windowMs: 60_000,
      description: '1,000 dashboard requests/minute',
    },
    analytics: {
      max: 25_000,
      windowMs: 60_000,
      description: '25,000 analytics events/minute',
    },
    transparency: {
      max: 500,
      windowMs: 60_000,
      description: '500 transparency API requests/minute',
    },
    rtb: {
      max: 5_000,
      windowMs: 60_000,
      description: '5,000 RTB/SSAI requests/minute',
    },
  },

  scale: {
    global: {
      max: 20_000,
      windowMs: 60_000,
      description: '20,000 requests/minute global',
    },
    auth: {
      max: 100,
      windowMs: 60_000,
      description: '100 auth requests/minute',
    },
    dashboard: {
      max: 3_000,
      windowMs: 60_000,
      description: '3,000 dashboard requests/minute',
    },
    analytics: {
      max: 100_000,
      windowMs: 60_000,
      description: '100,000 analytics events/minute',
    },
    transparency: {
      max: 2_000,
      windowMs: 60_000,
      description: '2,000 transparency API requests/minute',
    },
    rtb: {
      max: 20_000,
      windowMs: 60_000,
      description: '20,000 RTB/SSAI requests/minute',
    },
  },

  enterprise: {
    global: {
      max: 100_000,
      windowMs: 60_000,
      description: '100,000 requests/minute global',
    },
    auth: {
      max: 200,
      windowMs: 60_000,
      description: '200 auth requests/minute',
    },
    dashboard: {
      max: 10_000,
      windowMs: 60_000,
      description: '10,000 dashboard requests/minute',
    },
    analytics: {
      max: 500_000,
      windowMs: 60_000,
      description: '500,000 analytics events/minute',
    },
    transparency: {
      max: 10_000,
      windowMs: 60_000,
      description: '10,000 transparency API requests/minute',
    },
    rtb: {
      max: 100_000,
      windowMs: 60_000,
      description: '100,000 RTB/SSAI requests/minute',
    },
  },
};

/**
 * Default rate limits for unauthenticated requests
 * More restrictive than Starter tier to prevent abuse
 */
export const UNAUTHENTICATED_RATE_LIMITS: TierRateLimits = {
  global: {
    max: 300,
    windowMs: 60_000,
    description: '300 requests/minute for unauthenticated clients',
  },
  auth: {
    max: 10,
    windowMs: 60_000,
    description: '10 auth requests/minute (strict brute-force protection)',
  },
  dashboard: {
    max: 60,
    windowMs: 60_000,
    description: '60 dashboard requests/minute (read-only public data)',
  },
  analytics: {
    max: 1_000,
    windowMs: 60_000,
    description: '1,000 analytics events/minute (SDK with app key)',
  },
  transparency: {
    max: 30,
    windowMs: 60_000,
    description: '30 transparency API requests/minute',
  },
  rtb: {
    max: 500,
    windowMs: 60_000,
    description: '500 RTB/SSAI requests/minute',
  },
};

/**
 * Per-endpoint specific limits (overrides tier limits)
 * These are security-sensitive or resource-intensive endpoints
 */
export const ENDPOINT_SPECIFIC_LIMITS: Record<string, RateLimitConfig> = {
  // Security-sensitive endpoints
  '/auth/login': {
    max: 5,
    windowMs: 60_000,
    description: '5 login attempts/minute per IP',
  },
  '/auth/register': {
    max: 3,
    windowMs: 300_000, // 5 minutes
    description: '3 registrations per 5 minutes per IP',
  },
  '/auth/password-reset': {
    max: 3,
    windowMs: 300_000,
    description: '3 password reset requests per 5 minutes per IP',
  },
  '/auth/2fa/verify': {
    max: 5,
    windowMs: 60_000,
    description: '5 2FA attempts/minute per IP',
  },
  
  // Resource-intensive endpoints
  '/data-export': {
    max: 5,
    windowMs: 3600_000, // 1 hour
    description: '5 data exports per hour',
  },
  '/reports/generate': {
    max: 10,
    windowMs: 3600_000,
    description: '10 report generations per hour',
  },
  
  // High-cost operations
  '/migration/import': {
    max: 10,
    windowMs: 3600_000,
    description: '10 migration imports per hour',
  },
};

/**
 * Routes exempt from rate limiting
 * These are either internally protected or need high availability
 */
export const RATE_LIMIT_EXEMPT_ROUTES = [
  '/health',
  '/ready',
  '/live',
  '/metrics',
  '/status.json',
  '/slo.json',
];

/**
 * Get rate limits for a specific tier
 */
export function getRateLimitsForTier(tier: PlatformTierId | null): TierRateLimits {
  if (!tier) {
    return UNAUTHENTICATED_RATE_LIMITS;
  }
  return TIER_RATE_LIMITS[tier] || TIER_RATE_LIMITS.starter;
}

/**
 * Get endpoint-specific limit if exists, otherwise return null
 */
export function getEndpointLimit(path: string): RateLimitConfig | null {
  // Normalize path for matching
  const normalizedPath = path.replace(/\/api\/v\d+/, '').replace(/\/$/, '');
  return ENDPOINT_SPECIFIC_LIMITS[normalizedPath] || null;
}

/**
 * Check if a route is exempt from rate limiting
 */
export function isRateLimitExempt(path: string): boolean {
  const normalizedPath = path.replace(/\/api\/v\d+/, '');
  return RATE_LIMIT_EXEMPT_ROUTES.some(exempt => normalizedPath.startsWith(exempt));
}

/**
 * Environment variable overrides
 * Allows runtime tuning without code changes
 */
export function getEnvOverrides() {
  return {
    globalMax: parseInt(process.env.RATE_LIMIT_GLOBAL_MAX || '', 10) || null,
    globalWindowMs: parseInt(process.env.RATE_LIMIT_GLOBAL_WINDOW_MS || '', 10) || null,
    authMax: parseInt(process.env.RATE_LIMIT_AUTH_MAX || '', 10) || null,
    authWindowMs: parseInt(process.env.RATE_LIMIT_AUTH_WINDOW_MS || '', 10) || null,
    analyticsMax: parseInt(process.env.RATE_LIMIT_ANALYTICS_MAX || '', 10) || null,
  };
}
