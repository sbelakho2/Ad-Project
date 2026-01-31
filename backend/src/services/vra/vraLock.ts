/**
 * VRA Distributed Locking Service
 * 
 * Provides Redis-based distributed locks to prevent race conditions
 * in multi-instance deployments during:
 * - Network sync operations (prevents duplicate revenue inserts)
 * - Reconciliation runs (prevents conflicting calculations)
 * - Proof issuance (prevents duplicate merkle root generation)
 * 
 * Uses Redlock algorithm pattern for distributed lock safety.
 */

import { redis } from '../../utils/redis';
import logger from '../../utils/logger';
import crypto from 'crypto';

const LOCK_PREFIX = 'vra:lock:';
const DEFAULT_LOCK_TTL_SECONDS = 60; // 60 seconds default
const MAX_LOCK_TTL_SECONDS = 10 * 60; // 10 minutes max
const LOCK_RETRY_DELAY_MS = 100;
const MAX_LOCK_RETRIES = 50;

export interface LockOptions {
  /** Time-to-live in seconds (default: 60) */
  ttlSeconds?: number;
  /** Maximum retries to acquire lock (default: 50) */
  maxRetries?: number;
  /** Delay between retries in milliseconds (default: 100) */
  retryDelayMs?: number;
}

export interface AcquiredLock {
  key: string;
  value: string;
  ttlSeconds: number;
  acquiredAt: number;
}

/**
 * Helper function to sleep for a given duration
 */
function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Acquire a distributed lock
 * 
 * @param lockName - Unique name for the lock (e.g., 'sync:connection:abc123')
 * @param options - Lock configuration options
 * @returns The acquired lock object, or null if lock couldn't be acquired
 */
export async function acquireLock(
  lockName: string,
  options?: LockOptions
): Promise<AcquiredLock | null> {
  const ttlSeconds = Math.min(options?.ttlSeconds || DEFAULT_LOCK_TTL_SECONDS, MAX_LOCK_TTL_SECONDS);
  const maxRetries = options?.maxRetries ?? MAX_LOCK_RETRIES;
  const retryDelayMs = options?.retryDelayMs ?? LOCK_RETRY_DELAY_MS;
  
  const key = `${LOCK_PREFIX}${lockName}`;
  const value = generateLockValue();
  
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      // Use setIfNotExists for atomic lock acquisition
      const acquired = await redis.setIfNotExists(key, value, ttlSeconds);
      
      if (acquired === true) {
        logger.debug('VRA Lock: Acquired', { lockName, attempt, ttlSeconds });
        return {
          key,
          value,
          ttlSeconds,
          acquiredAt: Date.now(),
        };
      }
      
      // Lock not acquired (false or null), wait and retry
      if (attempt < maxRetries - 1) {
        await sleep(retryDelayMs);
      }
    } catch (err) {
      logger.warn('VRA Lock: Error during acquisition attempt', {
        lockName,
        attempt,
        error: (err as Error).message,
      });
      
      if (attempt < maxRetries - 1) {
        await sleep(retryDelayMs);
      }
    }
  }
  
  logger.warn('VRA Lock: Failed to acquire after max retries', { lockName, maxRetries });
  return null;
}

/**
 * Release a distributed lock
 * 
 * Gets current value and compares before deleting to prevent
 * accidentally releasing a lock held by another process.
 * 
 * @param lock - The lock object returned from acquireLock
 * @returns true if lock was released, false otherwise
 */
export async function releaseLock(lock: AcquiredLock): Promise<boolean> {
  try {
    // Get current value and compare before deleting
    const currentValue = await redis.get<string>(lock.key);
    
    if (currentValue === lock.value) {
      await redis.del(lock.key);
      logger.debug('VRA Lock: Released', { lockName: lock.key });
      return true;
    } else if (currentValue === null) {
      // Lock already expired
      logger.debug('VRA Lock: Already expired', { lockName: lock.key });
      return true;
    } else {
      // Lock was taken by someone else (shouldn't happen if TTL is set correctly)
      logger.warn('VRA Lock: Value mismatch, not releasing', {
        lockName: lock.key,
        expectedValue: lock.value,
        actualValue: currentValue,
      });
      return false;
    }
  } catch (err) {
    logger.error('VRA Lock: Error releasing', {
      lockName: lock.key,
      error: (err as Error).message,
    });
    return false;
  }
}

/**
 * Extend the TTL of an existing lock
 * 
 * Useful for long-running operations that need more time.
 * 
 * @param lock - The lock object to extend
 * @param additionalSeconds - Additional seconds to add to TTL
 * @returns true if extended, false otherwise
 */
export async function extendLock(lock: AcquiredLock, additionalSeconds: number): Promise<boolean> {
  try {
    const currentValue = await redis.get<string>(lock.key);
    
    if (currentValue !== lock.value) {
      logger.warn('VRA Lock: Cannot extend - value mismatch', { lockName: lock.key });
      return false;
    }
    
    const newTtlSeconds = lock.ttlSeconds + additionalSeconds;
    // Use expire to extend the TTL
    const extended = await redis.expire(lock.key, newTtlSeconds);
    
    if (extended) {
      lock.ttlSeconds = newTtlSeconds;
      logger.debug('VRA Lock: Extended', { lockName: lock.key, newTtlSeconds });
      return true;
    }
    
    return false;
  } catch (err) {
    logger.error('VRA Lock: Error extending', {
      lockName: lock.key,
      error: (err as Error).message,
    });
    return false;
  }
}

/**
 * Execute a function while holding a lock
 * 
 * Automatically acquires the lock, executes the function, and releases the lock.
 * Handles errors and ensures lock is released even if function throws.
 * 
 * @param lockName - Name of the lock to acquire
 * @param fn - Async function to execute while holding the lock
 * @param options - Lock options
 * @returns Result of the function, or throws if lock couldn't be acquired
 */
export async function withLock<T>(
  lockName: string,
  fn: () => Promise<T>,
  options?: LockOptions
): Promise<T> {
  const lock = await acquireLock(lockName, options);
  
  if (!lock) {
    throw new Error(`Failed to acquire lock: ${lockName}`);
  }
  
  try {
    return await fn();
  } finally {
    await releaseLock(lock);
  }
}

/**
 * Generate lock-specific keys for VRA operations
 */
export const lockKeys = {
  /** Lock for syncing a specific network connection */
  syncConnection: (connectionId: string) => `sync:connection:${connectionId}`,
  
  /** Lock for syncing all connections for a publisher */
  syncPublisher: (publisherId: string) => `sync:publisher:${publisherId}`,
  
  /** Lock for reconciliation run for a publisher */
  reconciliation: (publisherId: string, windowStart: string) => 
    `recon:${publisherId}:${windowStart}`,
  
  /** Lock for daily proof issuance */
  dailyProof: (day: string) => `proof:daily:${day}`,
  
  /** Lock for monthly digest issuance */
  monthlyDigest: (month: string) => `proof:monthly:${month}`,
  
  /** Lock for expected data build */
  expectedBuild: (windowStart: string, windowEnd: string) => 
    `expected:${windowStart}:${windowEnd}`,
};

/**
 * Generate a unique lock value
 * Combines process ID with random bytes for uniqueness across processes
 */
function generateLockValue(): string {
  const processId = process.pid.toString(36);
  const random = crypto.randomBytes(8).toString('hex');
  const timestamp = Date.now().toString(36);
  return `${processId}:${timestamp}:${random}`;
}
