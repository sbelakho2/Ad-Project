/**
 * AdCacheTTL - Computes ad cache TTL based on refresh interval.
 * 
 * The TTL is calculated as 2× the refresh interval, capped at 60 minutes.
 * This ensures ads remain cached long enough for the next refresh cycle
 * while preventing stale ads from lingering too long.
 */

/** Maximum TTL in milliseconds (60 minutes) */
export const MAX_TTL_MS = 60 * 60 * 1000;

/** Maximum TTL in seconds (60 minutes) */
export const MAX_TTL_SECONDS = 60 * 60;

/** Default refresh interval in milliseconds if none specified */
export const DEFAULT_REFRESH_INTERVAL_MS = 30 * 1000;

/** Minimum TTL in milliseconds (30 seconds) */
export const MIN_TTL_MS = 30 * 1000;

/**
 * Cache metadata for an ad entry.
 */
export interface AdCacheMetadata {
  /** Timestamp when the ad was cached (monotonic if available, Date.now otherwise) */
  cachedAtMs: number;
  /** TTL in milliseconds */
  ttlMs: number;
  /** Refresh interval in milliseconds */
  refreshIntervalMs: number;
  /** When this entry expires */
  expiresAtMs: number;
}

/**
 * Calculates the cache TTL based on the refresh interval.
 * 
 * @param refreshIntervalMs - The ad refresh interval in milliseconds
 * @returns The TTL in milliseconds (2× refreshInterval, capped at 60 minutes)
 */
export function calculateTTLMs(refreshIntervalMs: number): number {
  if (refreshIntervalMs <= 0) {
    return Math.min(DEFAULT_REFRESH_INTERVAL_MS * 2, MAX_TTL_MS);
  }
  const ttl = refreshIntervalMs * 2;
  return Math.max(MIN_TTL_MS, Math.min(ttl, MAX_TTL_MS));
}

/**
 * Calculates the cache TTL based on the refresh interval in seconds.
 * 
 * @param refreshIntervalSeconds - The ad refresh interval in seconds
 * @returns The TTL in seconds (2× refreshInterval, capped at 60 minutes)
 */
export function calculateTTLSeconds(refreshIntervalSeconds: number): number {
  if (refreshIntervalSeconds <= 0) {
    return Math.min((DEFAULT_REFRESH_INTERVAL_MS / 1000) * 2, MAX_TTL_SECONDS);
  }
  const ttl = refreshIntervalSeconds * 2;
  return Math.max(30, Math.min(ttl, MAX_TTL_SECONDS));
}

/**
 * Calculates the effective TTL, respecting any server-provided TTL.
 * 
 * If the server provides a TTL, we use the minimum of:
 * - Server-provided TTL
 * - Calculated TTL (2× refreshInterval)
 * - Maximum TTL (60 minutes)
 * 
 * @param refreshIntervalMs - The ad refresh interval in milliseconds
 * @param serverTTLSeconds - The TTL provided by the server (optional)
 * @returns The effective TTL in milliseconds
 */
export function calculateEffectiveTTLMs(
  refreshIntervalMs: number,
  serverTTLSeconds?: number
): number {
  const calculatedTTL = calculateTTLMs(refreshIntervalMs);
  
  if (serverTTLSeconds !== undefined && serverTTLSeconds > 0) {
    const serverTTLMs = serverTTLSeconds * 1000;
    return Math.min(calculatedTTL, Math.min(serverTTLMs, MAX_TTL_MS));
  }
  
  return calculatedTTL;
}

/**
 * Checks if an ad has expired based on its cache timestamp and TTL.
 * 
 * @param cachedAtMs - The timestamp when the ad was cached
 * @param ttlMs - The TTL in milliseconds
 * @param nowMs - The current time
 * @returns True if the ad has expired
 */
export function isExpired(cachedAtMs: number, ttlMs: number, nowMs: number): boolean {
  return nowMs >= cachedAtMs + ttlMs;
}

/**
 * Gets the remaining TTL for a cached ad.
 * 
 * @param cachedAtMs - The timestamp when the ad was cached
 * @param ttlMs - The TTL in milliseconds
 * @param nowMs - The current time
 * @returns The remaining TTL in milliseconds (0 if expired)
 */
export function remainingTTLMs(cachedAtMs: number, ttlMs: number, nowMs: number): number {
  const expiresAt = cachedAtMs + ttlMs;
  return Math.max(0, expiresAt - nowMs);
}

/**
 * Determines if an ad should be pre-fetched based on remaining TTL.
 * Pre-fetch should occur when remaining TTL is less than the refresh interval.
 * 
 * @param cachedAtMs - The timestamp when the ad was cached
 * @param ttlMs - The TTL in milliseconds
 * @param refreshIntervalMs - The refresh interval in milliseconds
 * @param nowMs - The current time
 * @returns True if a pre-fetch should be triggered
 */
export function shouldPrefetch(
  cachedAtMs: number,
  ttlMs: number,
  refreshIntervalMs: number,
  nowMs: number
): boolean {
  const remaining = remainingTTLMs(cachedAtMs, ttlMs, nowMs);
  return remaining > 0 && remaining < refreshIntervalMs;
}

/**
 * Creates cache metadata for an ad entry.
 * 
 * @param refreshIntervalMs - The ad refresh interval in milliseconds
 * @param serverTTLSeconds - The TTL provided by the server (optional)
 * @param nowMs - The current timestamp (defaults to Date.now())
 * @returns The cache metadata
 */
export function createCacheMetadata(
  refreshIntervalMs: number,
  serverTTLSeconds?: number,
  nowMs: number = Date.now()
): AdCacheMetadata {
  const ttlMs = calculateEffectiveTTLMs(refreshIntervalMs, serverTTLSeconds);
  
  return {
    cachedAtMs: nowMs,
    ttlMs,
    refreshIntervalMs,
    expiresAtMs: nowMs + ttlMs,
  };
}

/**
 * Helper class for working with cached ads.
 */
export class AdCacheEntry {
  private readonly metadata: AdCacheMetadata;
  
  constructor(
    refreshIntervalMs: number,
    serverTTLSeconds?: number,
    nowMs: number = Date.now()
  ) {
    this.metadata = createCacheMetadata(refreshIntervalMs, serverTTLSeconds, nowMs);
  }
  
  /**
   * Gets the cache metadata.
   */
  getMetadata(): AdCacheMetadata {
    return { ...this.metadata };
  }
  
  /**
   * Checks if this entry has expired.
   */
  isExpired(nowMs: number = Date.now()): boolean {
    return isExpired(this.metadata.cachedAtMs, this.metadata.ttlMs, nowMs);
  }
  
  /**
   * Gets the remaining TTL in milliseconds.
   */
  remainingMs(nowMs: number = Date.now()): number {
    return remainingTTLMs(this.metadata.cachedAtMs, this.metadata.ttlMs, nowMs);
  }
  
  /**
   * Checks if this ad should be pre-fetched.
   */
  shouldPrefetch(nowMs: number = Date.now()): boolean {
    return shouldPrefetch(
      this.metadata.cachedAtMs,
      this.metadata.ttlMs,
      this.metadata.refreshIntervalMs,
      nowMs
    );
  }
}

export default {
  MAX_TTL_MS,
  MAX_TTL_SECONDS,
  DEFAULT_REFRESH_INTERVAL_MS,
  MIN_TTL_MS,
  calculateTTLMs,
  calculateTTLSeconds,
  calculateEffectiveTTLMs,
  isExpired,
  remainingTTLMs,
  shouldPrefetch,
  createCacheMetadata,
  AdCacheEntry,
};
