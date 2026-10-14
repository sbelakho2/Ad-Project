import {
  calculateTTLMs,
  calculateTTLSeconds,
  calculateEffectiveTTLMs,
  isExpired,
  remainingTTLMs,
  shouldPrefetch,
  createCacheMetadata,
  AdCacheEntry,
  MAX_TTL_MS,
  MAX_TTL_SECONDS,
  DEFAULT_REFRESH_INTERVAL_MS,
  MIN_TTL_MS,
} from '../src/adCacheTTL';

describe('AdCacheTTL', () => {
  describe('calculateTTLMs', () => {
    it('should return 2x refresh interval', () => {
      expect(calculateTTLMs(30_000)).toBe(60_000);
      expect(calculateTTLMs(60_000)).toBe(120_000);
    });

    it('should cap at MAX_TTL_MS (60 minutes)', () => {
      expect(calculateTTLMs(45 * 60 * 1000)).toBe(MAX_TTL_MS); // 45 min * 2 = 90 min, capped to 60
    });

    it('should enforce minimum TTL', () => {
      expect(calculateTTLMs(10_000)).toBe(MIN_TTL_MS); // 10s * 2 = 20s, but min is 30s
    });

    it('should use default for zero or negative interval', () => {
      expect(calculateTTLMs(0)).toBe(DEFAULT_REFRESH_INTERVAL_MS * 2);
      expect(calculateTTLMs(-1000)).toBe(DEFAULT_REFRESH_INTERVAL_MS * 2);
    });
  });

  describe('calculateTTLSeconds', () => {
    it('should return 2x refresh interval in seconds', () => {
      expect(calculateTTLSeconds(30)).toBe(60);
      expect(calculateTTLSeconds(60)).toBe(120);
    });

    it('should cap at MAX_TTL_SECONDS (60 minutes)', () => {
      expect(calculateTTLSeconds(45 * 60)).toBe(MAX_TTL_SECONDS);
    });

    it('should enforce minimum TTL (30 seconds)', () => {
      expect(calculateTTLSeconds(10)).toBe(30); // 10s * 2 = 20s, but min is 30s
    });

    it('should use default for zero or negative interval', () => {
      expect(calculateTTLSeconds(0)).toBe((DEFAULT_REFRESH_INTERVAL_MS / 1000) * 2);
      expect(calculateTTLSeconds(-10)).toBe((DEFAULT_REFRESH_INTERVAL_MS / 1000) * 2);
    });
  });

  describe('calculateEffectiveTTLMs', () => {
    it('should use calculated TTL when no server TTL provided', () => {
      expect(calculateEffectiveTTLMs(30_000)).toBe(60_000);
    });

    it('should use calculated TTL when server TTL is undefined', () => {
      expect(calculateEffectiveTTLMs(30_000, undefined)).toBe(60_000);
    });

    it('should use minimum of calculated and server TTL', () => {
      // Server TTL (45s) < calculated (60s)
      expect(calculateEffectiveTTLMs(30_000, 45)).toBe(45_000);
      
      // Server TTL (90s) > calculated (60s)
      expect(calculateEffectiveTTLMs(30_000, 90)).toBe(60_000);
    });

    it('should respect MAX_TTL_MS even with large server TTL', () => {
      const largeServerTTL = 120 * 60; // 120 minutes
      const largeRefreshInterval = 60 * 60 * 1000; // 60 min
      
      expect(calculateEffectiveTTLMs(largeRefreshInterval, largeServerTTL)).toBe(MAX_TTL_MS);
    });

    it('should ignore zero or negative server TTL', () => {
      expect(calculateEffectiveTTLMs(30_000, 0)).toBe(60_000);
      expect(calculateEffectiveTTLMs(30_000, -10)).toBe(60_000);
    });
  });

  describe('isExpired', () => {
    it('should return false when not expired', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const now = 30_000;
      
      expect(isExpired(cachedAt, ttl, now)).toBe(false);
    });

    it('should return true when expired', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const now = 70_000;
      
      expect(isExpired(cachedAt, ttl, now)).toBe(true);
    });

    it('should return true when exactly at expiry', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const now = 61_000;
      
      expect(isExpired(cachedAt, ttl, now)).toBe(true);
    });
  });

  describe('remainingTTLMs', () => {
    it('should return remaining time', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const now = 30_000;
      
      expect(remainingTTLMs(cachedAt, ttl, now)).toBe(31_000);
    });

    it('should return 0 when expired', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const now = 70_000;
      
      expect(remainingTTLMs(cachedAt, ttl, now)).toBe(0);
    });

    it('should return 0 when exactly at expiry', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const now = 61_000;
      
      expect(remainingTTLMs(cachedAt, ttl, now)).toBe(0);
    });
  });

  describe('shouldPrefetch', () => {
    it('should return true when remaining TTL is less than refresh interval', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const refreshInterval = 30_000;
      const now = 40_000; // 22s remaining, less than 30s refresh
      
      expect(shouldPrefetch(cachedAt, ttl, refreshInterval, now)).toBe(true);
    });

    it('should return false when remaining TTL is greater than refresh interval', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const refreshInterval = 30_000;
      const now = 10_000; // 51s remaining, more than 30s refresh
      
      expect(shouldPrefetch(cachedAt, ttl, refreshInterval, now)).toBe(false);
    });

    it('should return false when already expired', () => {
      const cachedAt = 1000;
      const ttl = 60_000;
      const refreshInterval = 30_000;
      const now = 70_000;
      
      expect(shouldPrefetch(cachedAt, ttl, refreshInterval, now)).toBe(false);
    });
  });

  describe('createCacheMetadata', () => {
    it('should create metadata with calculated TTL', () => {
      const now = 1000;
      const metadata = createCacheMetadata(30_000, undefined, now);
      
      expect(metadata.cachedAtMs).toBe(1000);
      expect(metadata.ttlMs).toBe(60_000);
      expect(metadata.refreshIntervalMs).toBe(30_000);
      expect(metadata.expiresAtMs).toBe(61_000);
    });

    it('should respect server TTL', () => {
      const now = 1000;
      const metadata = createCacheMetadata(30_000, 45, now);
      
      expect(metadata.ttlMs).toBe(45_000);
      expect(metadata.expiresAtMs).toBe(46_000);
    });
  });

  describe('AdCacheEntry', () => {
    it('should create entry with metadata', () => {
      const entry = new AdCacheEntry(30_000, undefined, 1000);
      const metadata = entry.getMetadata();
      
      expect(metadata.cachedAtMs).toBe(1000);
      expect(metadata.ttlMs).toBe(60_000);
    });

    it('should return copy of metadata', () => {
      const entry = new AdCacheEntry(30_000, undefined, 1000);
      const metadata1 = entry.getMetadata();
      const metadata2 = entry.getMetadata();
      
      expect(metadata1).not.toBe(metadata2);
      expect(metadata1).toEqual(metadata2);
    });

    it('should check expiration', () => {
      const entry = new AdCacheEntry(30_000, undefined, 1000);
      
      expect(entry.isExpired(30_000)).toBe(false);
      expect(entry.isExpired(70_000)).toBe(true);
    });

    it('should return remaining TTL', () => {
      const entry = new AdCacheEntry(30_000, undefined, 1000);
      
      expect(entry.remainingMs(30_000)).toBe(31_000);
      expect(entry.remainingMs(70_000)).toBe(0);
    });

    it('should check prefetch status', () => {
      const entry = new AdCacheEntry(30_000, undefined, 1000);
      
      // 22s remaining < 30s refresh → should prefetch
      expect(entry.shouldPrefetch(40_000)).toBe(true);
      
      // 51s remaining > 30s refresh → should not prefetch
      expect(entry.shouldPrefetch(10_000)).toBe(false);
    });
  });

  describe('constants', () => {
    it('should have correct MAX_TTL values', () => {
      expect(MAX_TTL_MS).toBe(60 * 60 * 1000);
      expect(MAX_TTL_SECONDS).toBe(60 * 60);
    });

    it('should have correct DEFAULT_REFRESH_INTERVAL_MS', () => {
      expect(DEFAULT_REFRESH_INTERVAL_MS).toBe(30 * 1000);
    });

    it('should have correct MIN_TTL_MS', () => {
      expect(MIN_TTL_MS).toBe(30 * 1000);
    });
  });
});
