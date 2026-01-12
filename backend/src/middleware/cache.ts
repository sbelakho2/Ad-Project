/**
 * Cache Middleware
 * 
 * Express middleware for automatic response caching with Redis
 * Provides flexible caching strategies for different endpoints
 */

import { Request, Response, NextFunction } from 'express';
import { redis } from '../utils/redis';
import logger from '../utils/logger';

export interface CacheOptions {
  ttl?: number;                    // Cache TTL in seconds
  keyGenerator?: (req: Request) => string;  // Custom cache key generator
  condition?: (req: Request) => boolean;    // Condition to cache response
  varyBy?: string[];               // Request properties to vary cache by (e.g., ['publisherId', 'query.startDate'])
}

/**
 * Generate default cache key from request
 */
function generateCacheKey(req: Request, options?: CacheOptions): string {
  const publisherId = req.user?.publisherId || 'anonymous';
  const path = req.path;
  const method = req.method;
  
  const keyParts = [`cache:${method}:${path}:${publisherId}`];
  
  // Add vary-by parameters
  if (options?.varyBy) {
    for (const varyKey of options.varyBy) {
      const value = getNestedProperty(req, varyKey);
      if (value !== undefined) {
        keyParts.push(`${varyKey}:${value}`);
      }
    }
  } else {
    // Default: vary by all query parameters
    const queryString = JSON.stringify(req.query);
    if (queryString !== '{}') {
      keyParts.push(queryString);
    }
  }
  
  return keyParts.join(':');
}

/**
 * Get nested property from object using dot notation
 */
function getNestedProperty<T>(obj: T, path: string): unknown {
  return path.split('.').reduce<unknown>((current, key) => {
    if (current === undefined || current === null || typeof current !== 'object') {
      return undefined;
    }

    if (Array.isArray(current)) {
      const index = Number(key);
      return Number.isNaN(index) ? undefined : current[index];
    }

    return (current as Record<string, unknown>)[key];
  }, obj as unknown);
}

/**
 * Cache middleware factory
 * 
 * @example
 * // Cache for 5 minutes
 * router.get('/analytics', cache({ ttl: 300 }), analyticsController.getMetrics);
 * 
 * // Cache with custom key
 * router.get('/data', cache({ 
 *   ttl: 600,
 *   keyGenerator: (req) => `custom:${req.params.id}:${req.query.type}`
 * }), dataController.getData);
 */
export function cache(options: CacheOptions = {}): (req: Request, res: Response, next: NextFunction) => void {
  return async (req: Request, res: Response, next: NextFunction) => {
    // Skip caching for non-GET requests
    if (req.method !== 'GET') {
      return next();
    }

    // Check condition if provided
    if (options.condition && !options.condition(req)) {
      return next();
    }

    // Skip if Redis is not connected
    if (!redis.isReady()) {
      logger.debug('Redis not ready, skipping cache check');
      return next();
    }

    try {
      // Generate cache key
      const cacheKey = options.keyGenerator 
        ? options.keyGenerator(req)
        : generateCacheKey(req, options);

      // Check cache
      const cachedData = await redis.get(cacheKey);
      
      if (cachedData !== null) {
        logger.debug('Cache hit', { key: cacheKey });
        
        // Set cache header
        res.setHeader('X-Cache', 'HIT');
        
        return res.json(cachedData);
      }

      logger.debug('Cache miss', { key: cacheKey });
      
      // Set cache header
      res.setHeader('X-Cache', 'MISS');

      // Intercept res.json to cache the response
      const originalJson = res.json.bind(res);
      
      res.json = function json<T>(data: T) {
        // Only cache successful responses
        if (res.statusCode >= 200 && res.statusCode < 300) {
          const ttl = options.ttl || 300; // Default 5 minutes
          redis.set(cacheKey, data, ttl).catch((err) => {
            logger.error('Failed to cache response', { error: err, key: cacheKey });
          });
        }
        
        return originalJson(data);
      } as typeof res.json;

      next();
    } catch (error) {
      logger.error('Cache middleware error', { error });
      // Continue without caching on error
      next();
    }
  };
}

/**
 * Cache invalidation middleware
 * Clears cache entries matching a pattern after successful mutations
 * 
 * @example
 * router.post('/experiments', invalidateCache('ab:experiments:*'), createExperiment);
 */
export function invalidateCache(pattern: string | ((req: Request) => string)): (req: Request, res: Response, next: NextFunction) => void {
  return async (req: Request, res: Response, next: NextFunction) => {
    // Store original send function
    const originalSend = res.send.bind(res);
    
    res.send = function send<T>(data: T) {
      // Only invalidate on successful mutations (2xx status codes)
      if (res.statusCode >= 200 && res.statusCode < 300) {
        const invalidationPattern = typeof pattern === 'function' 
          ? pattern(req)
          : pattern;
        
        redis.delPattern(invalidationPattern).then((count) => {
          // Cache invalidated silently
        }).catch((err) => {
          logger.error('Cache invalidation error', { error: err, pattern: invalidationPattern });
        });
      }
      
      return originalSend(data);
    } as typeof res.send;
    
    next();
  };
}

/**
 * Publisher-scoped cache invalidation
 * Clears all cache entries for a specific publisher
 */
export function invalidatePublisherCache(publisherId?: string): (req: Request, res: Response, next: NextFunction) => void {
  return invalidateCache((req) => {
    const sdkPublisherId = (req as any)?.sdk?.publisherId as string | undefined;
    const pubId = publisherId || req.user?.publisherId || sdkPublisherId || '*';
    return `*:${pubId}:*`;
  });
}

/**
 * Conditional cache middleware
 * Only caches based on query parameter or condition
 * 
 * @example
 * // Only cache if ?cache=true is present
 * router.get('/data', conditionalCache({ ttl: 600 }), getData);
 */
export function conditionalCache(options: CacheOptions = {}): (req: Request, res: Response, next: NextFunction) => void {
  return cache({
    ...options,
    condition: (req) => {
      // Check if caching is explicitly enabled via query param
      if (req.query.cache === 'true') {
        return true;
      }
      // Check custom condition
      if (options.condition) {
        return options.condition(req);
      }
      return false;
    },
  });
}

export default cache;
