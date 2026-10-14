using System;

namespace RivalApexMediation.Core.Cache
{
    /// <summary>
    /// AdCacheTTL - Computes ad cache TTL based on refresh interval.
    /// 
    /// The TTL is calculated as 2× the refresh interval, capped at 60 minutes.
    /// This ensures ads remain cached long enough for the next refresh cycle
    /// while preventing stale ads from lingering too long.
    /// </summary>
    public static class AdCacheTTL
    {
        /// <summary>
        /// Maximum TTL in seconds (60 minutes)
        /// </summary>
        public const int MaxTTLSeconds = 60 * 60;
        
        /// <summary>
        /// Maximum TTL in milliseconds (60 minutes)
        /// </summary>
        public const long MaxTTLMs = 60L * 60L * 1000L;
        
        /// <summary>
        /// Default refresh interval in seconds if none specified
        /// </summary>
        public const int DefaultRefreshIntervalSeconds = 30;
        
        /// <summary>
        /// Minimum TTL in seconds (30 seconds)
        /// </summary>
        public const int MinTTLSeconds = 30;
        
        /// <summary>
        /// Calculates the cache TTL based on the refresh interval in seconds.
        /// </summary>
        /// <param name="refreshIntervalSeconds">The ad refresh interval in seconds</param>
        /// <returns>The TTL in seconds (2× refreshInterval, capped at 60 minutes)</returns>
        public static int CalculateTTLSeconds(int refreshIntervalSeconds)
        {
            if (refreshIntervalSeconds <= 0)
            {
                return Math.Min(DefaultRefreshIntervalSeconds * 2, MaxTTLSeconds);
            }
            
            int ttl = refreshIntervalSeconds * 2;
            return Math.Max(MinTTLSeconds, Math.Min(ttl, MaxTTLSeconds));
        }
        
        /// <summary>
        /// Calculates the cache TTL based on the refresh interval in milliseconds.
        /// </summary>
        /// <param name="refreshIntervalMs">The ad refresh interval in milliseconds</param>
        /// <returns>The TTL in milliseconds (2× refreshInterval, capped at 60 minutes)</returns>
        public static long CalculateTTLMs(long refreshIntervalMs)
        {
            if (refreshIntervalMs <= 0)
            {
                return Math.Min(DefaultRefreshIntervalSeconds * 2L * 1000L, MaxTTLMs);
            }
            
            long ttl = refreshIntervalMs * 2;
            long minTTLMs = MinTTLSeconds * 1000L;
            return Math.Max(minTTLMs, Math.Min(ttl, MaxTTLMs));
        }
        
        /// <summary>
        /// Calculates the effective TTL, respecting any server-provided TTL.
        /// </summary>
        /// <param name="refreshIntervalSeconds">The ad refresh interval in seconds</param>
        /// <param name="serverTTLSeconds">The TTL provided by the server (null if not provided)</param>
        /// <returns>The effective TTL in seconds</returns>
        public static int CalculateEffectiveTTLSeconds(int refreshIntervalSeconds, int? serverTTLSeconds)
        {
            int calculatedTTL = CalculateTTLSeconds(refreshIntervalSeconds);
            
            if (serverTTLSeconds.HasValue && serverTTLSeconds.Value > 0)
            {
                return Math.Min(calculatedTTL, Math.Min(serverTTLSeconds.Value, MaxTTLSeconds));
            }
            
            return calculatedTTL;
        }
        
        /// <summary>
        /// Checks if an ad has expired based on its cache timestamp and TTL.
        /// </summary>
        /// <param name="cachedAtMs">The timestamp when the ad was cached (monotonic)</param>
        /// <param name="ttlMs">The TTL in milliseconds</param>
        /// <param name="nowMs">The current time (monotonic)</param>
        /// <returns>True if the ad has expired</returns>
        public static bool IsExpired(long cachedAtMs, long ttlMs, long nowMs)
        {
            return nowMs >= cachedAtMs + ttlMs;
        }
        
        /// <summary>
        /// Gets the remaining TTL for a cached ad.
        /// </summary>
        /// <param name="cachedAtMs">The timestamp when the ad was cached (monotonic)</param>
        /// <param name="ttlMs">The TTL in milliseconds</param>
        /// <param name="nowMs">The current time (monotonic)</param>
        /// <returns>The remaining TTL in milliseconds (0 if expired)</returns>
        public static long RemainingTTLMs(long cachedAtMs, long ttlMs, long nowMs)
        {
            long expiresAt = cachedAtMs + ttlMs;
            return Math.Max(0, expiresAt - nowMs);
        }
        
        /// <summary>
        /// Determines if an ad should be pre-fetched based on remaining TTL.
        /// Pre-fetch should occur when remaining TTL is less than the refresh interval.
        /// </summary>
        /// <param name="cachedAtMs">The timestamp when the ad was cached (monotonic)</param>
        /// <param name="ttlMs">The TTL in milliseconds</param>
        /// <param name="refreshIntervalMs">The refresh interval in milliseconds</param>
        /// <param name="nowMs">The current time (monotonic)</param>
        /// <returns>True if a pre-fetch should be triggered</returns>
        public static bool ShouldPrefetch(long cachedAtMs, long ttlMs, long refreshIntervalMs, long nowMs)
        {
            long remaining = RemainingTTLMs(cachedAtMs, ttlMs, nowMs);
            return remaining > 0 && remaining < refreshIntervalMs;
        }
    }
    
    /// <summary>
    /// Metadata for a cached ad entry with TTL information.
    /// </summary>
    public struct AdCacheMetadata
    {
        public long CachedAtMs { get; }
        public long TTLMs { get; }
        public long RefreshIntervalMs { get; }
        
        public AdCacheMetadata(long cachedAtMs, long ttlMs, long refreshIntervalMs)
        {
            CachedAtMs = cachedAtMs;
            TTLMs = ttlMs;
            RefreshIntervalMs = refreshIntervalMs;
        }
        
        /// <summary>
        /// Gets the expiration timestamp.
        /// </summary>
        public long ExpiresAtMs => CachedAtMs + TTLMs;
        
        /// <summary>
        /// Checks if this entry has expired.
        /// </summary>
        public bool IsExpired(long nowMs) => AdCacheTTL.IsExpired(CachedAtMs, TTLMs, nowMs);
        
        /// <summary>
        /// Gets the remaining TTL.
        /// </summary>
        public long RemainingMs(long nowMs) => AdCacheTTL.RemainingTTLMs(CachedAtMs, TTLMs, nowMs);
        
        /// <summary>
        /// Checks if this ad should be pre-fetched.
        /// </summary>
        public bool ShouldPrefetch(long nowMs) => 
            AdCacheTTL.ShouldPrefetch(CachedAtMs, TTLMs, RefreshIntervalMs, nowMs);
    }
}
