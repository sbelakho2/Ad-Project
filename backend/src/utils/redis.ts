/**
 * Redis Client Utility
 * 
 * Provides Redis connection management and caching utilities
 * for high-performance data caching across the application
 */

import { createClient, RedisClientType } from 'redis';
import logger from './logger';

class RedisClient {
  private client: RedisClientType | null = null;
  private isConnected: boolean = false;
  private reconnectAttempts: number = 0;
  private readonly maxReconnectAttempts: number = 10;

  /**
   * Initialize Redis connection
   */
  async connect(): Promise<void> {
    if (this.isConnected && this.client) {
      return;
    }

    try {
      const redisUrl = process.env.REDIS_URL || 'redis://localhost:6379';
      
      this.client = createClient({
        url: redisUrl,
        socket: {
          connectTimeout: 5000, // 5 second timeout for initial connection
          reconnectStrategy: (retries: number) => {
            if (retries > this.maxReconnectAttempts) {
              logger.error('Max Redis reconnection attempts reached');
              return false; // Return false to stop reconnection
            }
            // Exponential backoff: 100ms, 200ms, 400ms, 800ms, etc.
            const delay = Math.min(100 * Math.pow(2, retries), 5000);
            logger.warn(`Reconnecting to Redis in ${delay}ms (attempt ${retries})`);
            return delay;
          },
        },
      });

      this.client.on('error', (err) => {
        logger.error('Redis client error', { error: err });
        this.isConnected = false;
      });

      this.client.on('connect', () => {
        logger.info('Redis client connecting...');
      });

      this.client.on('ready', () => {
        logger.info('Redis client connected and ready');
        this.isConnected = true;
        this.reconnectAttempts = 0;
      });

      this.client.on('reconnecting', () => {
        this.reconnectAttempts++;
        logger.warn('Redis client reconnecting...', { attempt: this.reconnectAttempts });
      });

      this.client.on('end', () => {
        logger.info('Redis client connection closed');
        this.isConnected = false;
      });

      await this.client.connect();
    } catch (error) {
      logger.error('Failed to connect to Redis', { error });
      this.isConnected = false;
      // Don't throw error - allow app to continue without Redis
    }
  }

  /**
   * Disconnect from Redis
   */
  async disconnect(): Promise<void> {
    try {
      if (this.client) {
        if (this.isConnected) {
          await this.client.quit();
        } else {
          // Force disconnect if not properly connected
          await this.client.disconnect();
        }
        this.client = null;
        this.isConnected = false;
        logger.info('Redis client disconnected');
      }
    } catch (error) {
      logger.warn('Error disconnecting from Redis', { error });
      // Force null the client anyway
      this.client = null;
      this.isConnected = false;
    }
  }

  /**
   * Get value from cache
   */
  async get<T = unknown>(key: string): Promise<T | null> {
    if (!this.client || !this.isConnected) {
      logger.warn('Redis not connected, skipping cache get', { key });
      return null;
    }

    try {
      const value = await this.client.get(key);
      if (value === null) {
        return null;
      }
      return JSON.parse(value) as T;
    } catch (error) {
      logger.error('Redis get error', { error, key });
      return null;
    }
  }

  /**
   * Set value in cache with optional TTL (in seconds)
   */
  async set(key: string, value: unknown, ttl?: number): Promise<boolean> {
    if (!this.client || !this.isConnected) {
      logger.warn('Redis not connected, skipping cache set', { key });
      return false;
    }

    try {
      const serialized = JSON.stringify(value);
      if (ttl) {
        await this.client.setEx(key, ttl, serialized);
      } else {
        await this.client.set(key, serialized);
      }
      return true;
    } catch (error) {
      logger.error('Redis set error', { error, key });
      return false;
    }
  }

  /**
   * Set key only if it does not already exist, with expiry (seconds).
   *
   * Returns:
   * - true  => key was set
   * - false => key already existed
   * - null  => Redis unavailable or error
   */
  async setIfNotExists(key: string, value: string, ttlSeconds: number): Promise<boolean | null> {
    if (!this.client || !this.isConnected) {
      return null;
    }

    try {
      const result = await this.client.set(key, value, { NX: true, EX: ttlSeconds });
      return result === 'OK';
    } catch (error) {
      logger.error('Redis setIfNotExists error', { error, key });
      return null;
    }
  }

  /**
   * Direct setEx helper for call sites that already serialize values
   */
  async setEx(key: string, ttl: number, value: string): Promise<boolean> {
    if (!this.client || !this.isConnected) {
      logger.warn('Redis not connected, skipping cache setEx', { key });
      return false;
    }

    try {
      await this.client.setEx(key, ttl, value);
      return true;
    } catch (error) {
      logger.error('Redis setEx error', { error, key });
      return false;
    }
  }

  /**
   * Get remaining time-to-live for a key in seconds
   */
  async ttl(key: string): Promise<number | null> {
    if (!this.client || !this.isConnected) {
      return null;
    }

    try {
      return await this.client.ttl(key);
    } catch (error) {
      logger.error('Redis TTL error', { error, key });
      return null;
    }
  }

  /**
   * Delete key from cache
   */
  async del(key: string): Promise<boolean> {
    if (!this.client || !this.isConnected) {
      logger.warn('Redis not connected, skipping cache delete', { key });
      return false;
    }

    try {
      await this.client.del(key);
      return true;
    } catch (error) {
      logger.error('Redis delete error', { error, key });
      return false;
    }
  }

  /**
   * Delete multiple keys matching a pattern
   */
  async delPattern(pattern: string): Promise<number> {
    if (!this.client || !this.isConnected) {
      logger.warn('Redis not connected, skipping pattern delete', { pattern });
      return 0;
    }

    try {
      const keys = await this.client.keys(pattern);
      if (keys.length === 0) {
        return 0;
      }
      await this.client.del(keys);
      return keys.length;
    } catch (error) {
      logger.error('Redis pattern delete error', { error, pattern });
      return 0;
    }
  }

  /**
   * Check if key exists
   */
  async exists(key: string): Promise<boolean> {
    if (!this.client || !this.isConnected) {
      return false;
    }

    try {
      const result = await this.client.exists(key);
      return result === 1;
    } catch (error) {
      logger.error('Redis exists error', { error, key });
      return false;
    }
  }

  /**
   * Set expiration time on key (in seconds)
   */
  async expire(key: string, seconds: number): Promise<boolean> {
    if (!this.client || !this.isConnected) {
      return false;
    }

    try {
      await this.client.expire(key, seconds);
      return true;
    } catch (error) {
      logger.error('Redis expire error', { error, key });
      return false;
    }
  }

  /**
   * Increment numeric value
   */
  async incr(key: string): Promise<number> {
    if (!this.client || !this.isConnected) {
      throw new Error('Redis not connected');
    }

    try {
      return await this.client.incr(key);
    } catch (error) {
      logger.error('Redis incr error', { error, key });
      throw error;
    }
  }

  /**
   * Get or set pattern - fetch from cache or compute and store
   */
  async getOrSet<T>(
    key: string,
    fetchFn: () => Promise<T>,
    ttl?: number
  ): Promise<T> {
    // Try to get from cache
    const cached = await this.get<T>(key);
    if (cached !== null) {
      return cached;
    }

    // Cache miss - fetch data
    const data = await fetchFn();
    
    // Store in cache
    await this.set(key, data, ttl);
    
    return data;
  }

  /**
   * Check if Redis is connected
   */
  isReady(): boolean {
    return this.isConnected && this.client !== null;
  }

  /**
   * Flush all keys (use with caution - only for testing)
   */
  async flushAll(): Promise<void> {
    if (!this.client || !this.isConnected) {
      return;
    }

    try {
      await this.client.flushAll();
      logger.warn('Redis cache flushed');
    } catch (error) {
      logger.error('Redis flush error', { error });
    }
  }
}

// Singleton instance
export const redis = new RedisClient();

// Cache key builders for consistent naming
export const cacheKeys = {
  // Analytics caching
  analytics: {
    revenue: (publisherId: string, startDate: string, endDate: string) =>
      `analytics:revenue:${publisherId}:${startDate}:${endDate}`,
    impressions: (publisherId: string, startDate: string, endDate: string) =>
      `analytics:impressions:${publisherId}:${startDate}:${endDate}`,
    performanceMetrics: (publisherId: string, startDate: string, endDate: string) =>
      `analytics:performance:${publisherId}:${startDate}:${endDate}`,
    adapterBreakdown: (publisherId: string, startDate: string, endDate: string) =>
      `analytics:adapter:${publisherId}:${startDate}:${endDate}`,
  },
  
  // SSAI session caching
  ssai: {
    session: (sessionId: string) =>
      `ssai:session:${sessionId}`,
    idempotency: (idempotencyKey: string) =>
      `ssai:idempotency:${idempotencyKey}`,
    tenantSession: (tenantId: string, sessionId: string) =>
      `ssai:tenant:${tenantId}:${sessionId}`,
  },
  
  // A/B Testing caching
  abTesting: {
    experiment: (experimentId: string) =>
      `ab:experiment:${experimentId}`,
    experimentList: (publisherId: string) =>
      `ab:experiments:${publisherId}`,
    metrics: (experimentId: string) =>
      `ab:metrics:${experimentId}`,
    significance: (experimentId: string, metric: string) =>
      `ab:significance:${experimentId}:${metric}`,
  },
  
  // Export jobs caching
  export: {
    job: (jobId: string) =>
      `export:job:${jobId}`,
    jobList: (publisherId: string) =>
      `export:jobs:${publisherId}`,
  },
  
  // Publisher data caching
  publisher: {
    profile: (publisherId: string) =>
      `publisher:${publisherId}`,
    adapters: (publisherId: string) =>
      `publisher:adapters:${publisherId}`,
  },
};

// Cache TTL constants (in seconds)
export const cacheTTL = {
  short: 60,           // 1 minute - for frequently changing data
  medium: 300,         // 5 minutes - for moderate update frequency
  long: 1800,          // 30 minutes - for relatively stable data
  veryLong: 3600,      // 1 hour - for rarely changing data
  day: 86400,          // 24 hours - for static/reference data
};

export default redis;
