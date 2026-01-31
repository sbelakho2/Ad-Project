/**
 * Rate Limiting Utility for Console API Routes (P1 Security Fix #2)
 * 
 * Simple in-memory rate limiter for Next.js API routes.
 * For production with multiple instances, consider using Redis.
 */

type RateLimitStore = Map<string, { count: number; resetTime: number }>;

const stores: Map<string, RateLimitStore> = new Map();

export interface RateLimitConfig {
  /** Window duration in milliseconds */
  windowMs: number;
  /** Maximum requests per window */
  max: number;
  /** Store identifier (for different limit types) */
  storeId?: string;
}

export interface RateLimitResult {
  success: boolean;
  limit: number;
  remaining: number;
  resetTime: number;
  retryAfterSeconds?: number;
}

/**
 * Get or create a rate limit store
 */
function getStore(storeId: string): RateLimitStore {
  if (!stores.has(storeId)) {
    stores.set(storeId, new Map());
  }
  return stores.get(storeId)!;
}

/**
 * Generate rate limit key from request
 */
export function getRateLimitKey(request: Request, identifier?: string): string {
  // Use X-Forwarded-For for proxied requests, fall back to IP
  const xff = request.headers.get('x-forwarded-for');
  const ip = xff ? xff.split(',')[0].trim() : 'unknown';
  return identifier ? `${identifier}:${ip}` : ip;
}

/**
 * Check rate limit for a key
 */
export function checkRateLimit(
  key: string,
  config: RateLimitConfig
): RateLimitResult {
  const { windowMs, max, storeId = 'default' } = config;
  const store = getStore(storeId);
  const now = Date.now();

  // Clean up expired entries periodically
  if (Math.random() < 0.1) {
    for (const [k, v] of store.entries()) {
      if (now >= v.resetTime) {
        store.delete(k);
      }
    }
  }

  const entry = store.get(key);
  
  if (!entry || now >= entry.resetTime) {
    // Window expired or new key - create fresh entry
    const resetTime = now + windowMs;
    store.set(key, { count: 1, resetTime });
    return {
      success: true,
      limit: max,
      remaining: max - 1,
      resetTime,
    };
  }

  if (entry.count >= max) {
    // Rate limited
    const retryAfterSeconds = Math.ceil((entry.resetTime - now) / 1000);
    return {
      success: false,
      limit: max,
      remaining: 0,
      resetTime: entry.resetTime,
      retryAfterSeconds,
    };
  }

  // Increment count
  entry.count++;
  return {
    success: true,
    limit: max,
    remaining: max - entry.count,
    resetTime: entry.resetTime,
  };
}

/**
 * Apply rate limit headers to response
 */
export function applyRateLimitHeaders(
  headers: Headers,
  result: RateLimitResult
): void {
  headers.set('X-RateLimit-Limit', result.limit.toString());
  headers.set('X-RateLimit-Remaining', result.remaining.toString());
  headers.set('X-RateLimit-Reset', Math.ceil(result.resetTime / 1000).toString());
  
  if (!result.success && result.retryAfterSeconds) {
    headers.set('Retry-After', result.retryAfterSeconds.toString());
  }
}

/**
 * Create a rate limited response
 */
export function createRateLimitResponse(result: RateLimitResult): Response {
  const headers = new Headers();
  applyRateLimitHeaders(headers, result);
  headers.set('Content-Type', 'application/json');

  return new Response(
    JSON.stringify({
      error: 'Too many requests',
      message: `Rate limit exceeded. Try again in ${result.retryAfterSeconds} seconds.`,
      retryAfterSeconds: result.retryAfterSeconds,
    }),
    {
      status: 429,
      headers,
    }
  );
}

// Pre-configured rate limiters for different use cases
export const RATE_LIMITS = {
  /** Auth endpoints: 5 requests per minute */
  auth: { windowMs: 60_000, max: 5, storeId: 'auth' } as RateLimitConfig,
  
  /** API endpoints: 100 requests per minute */
  api: { windowMs: 60_000, max: 100, storeId: 'api' } as RateLimitConfig,
  
  /** Strict: 10 requests per minute for sensitive operations */
  strict: { windowMs: 60_000, max: 10, storeId: 'strict' } as RateLimitConfig,
};
