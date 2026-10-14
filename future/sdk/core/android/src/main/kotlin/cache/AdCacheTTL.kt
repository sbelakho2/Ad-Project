package com.rivalapexmediation.sdk.cache

import kotlin.math.min

/**
 * AdCacheTTL - Computes ad cache TTL based on refresh interval.
 * 
 * The TTL is calculated as 2× the refresh interval, capped at 60 minutes.
 * This ensures ads remain cached long enough for the next refresh cycle
 * while preventing stale ads from lingering too long.
 */
object AdCacheTTL {
    
    /** Maximum TTL in milliseconds (60 minutes) */
    const val MAX_TTL_MS: Long = 60 * 60 * 1000L
    
    /** Maximum TTL in seconds (60 minutes) */
    const val MAX_TTL_SECONDS: Long = 60 * 60L
    
    /** Default refresh interval in milliseconds if none specified */
    const val DEFAULT_REFRESH_INTERVAL_MS: Long = 30 * 1000L
    
    /** Minimum TTL in milliseconds (30 seconds) */
    const val MIN_TTL_MS: Long = 30 * 1000L
    
    /**
     * Calculates the cache TTL based on the refresh interval.
     * 
     * @param refreshIntervalMs The ad refresh interval in milliseconds
     * @return The TTL in milliseconds (2× refreshInterval, capped at 60 minutes)
     */
    fun calculateTTLMs(refreshIntervalMs: Long): Long {
        if (refreshIntervalMs <= 0) {
            return min(DEFAULT_REFRESH_INTERVAL_MS * 2, MAX_TTL_MS)
        }
        val ttl = refreshIntervalMs * 2
        return ttl.coerceIn(MIN_TTL_MS, MAX_TTL_MS)
    }
    
    /**
     * Calculates the cache TTL based on the refresh interval in seconds.
     * 
     * @param refreshIntervalSeconds The ad refresh interval in seconds
     * @return The TTL in seconds (2× refreshInterval, capped at 60 minutes)
     */
    fun calculateTTLSeconds(refreshIntervalSeconds: Int): Int {
        if (refreshIntervalSeconds <= 0) {
            return min(DEFAULT_REFRESH_INTERVAL_MS / 1000 * 2, MAX_TTL_SECONDS).toInt()
        }
        val ttl = refreshIntervalSeconds * 2L
        return ttl.coerceIn(30L, MAX_TTL_SECONDS).toInt()
    }
    
    /**
     * Calculates the cache TTL, respecting any server-provided TTL.
     * 
     * If the server provides a TTL, we use the minimum of:
     * - Server-provided TTL
     * - Calculated TTL (2× refreshInterval)
     * - Maximum TTL (60 minutes)
     * 
     * @param refreshIntervalMs The ad refresh interval in milliseconds
     * @param serverTtlSeconds The TTL provided by the server (optional)
     * @return The effective TTL in milliseconds
     */
    fun calculateEffectiveTTLMs(refreshIntervalMs: Long, serverTtlSeconds: Int?): Long {
        val calculatedTtl = calculateTTLMs(refreshIntervalMs)
        
        return if (serverTtlSeconds != null && serverTtlSeconds > 0) {
            val serverTtlMs = serverTtlSeconds.toLong() * 1000L
            min(calculatedTtl, min(serverTtlMs, MAX_TTL_MS))
        } else {
            calculatedTtl
        }
    }
    
    /**
     * Checks if an ad has expired based on its cache timestamp and TTL.
     * 
     * @param cachedAtMs The timestamp when the ad was cached (monotonic)
     * @param ttlMs The TTL in milliseconds
     * @param nowMs The current time (monotonic)
     * @return True if the ad has expired
     */
    fun isExpired(cachedAtMs: Long, ttlMs: Long, nowMs: Long): Boolean {
        return nowMs >= cachedAtMs + ttlMs
    }
    
    /**
     * Gets the remaining TTL for a cached ad.
     * 
     * @param cachedAtMs The timestamp when the ad was cached (monotonic)
     * @param ttlMs The TTL in milliseconds
     * @param nowMs The current time (monotonic)
     * @return The remaining TTL in milliseconds (0 if expired)
     */
    fun remainingTTLMs(cachedAtMs: Long, ttlMs: Long, nowMs: Long): Long {
        val expiresAt = cachedAtMs + ttlMs
        return (expiresAt - nowMs).coerceAtLeast(0)
    }
    
    /**
     * Determines if an ad should be pre-fetched based on remaining TTL.
     * Pre-fetch should occur when remaining TTL is less than the refresh interval.
     * 
     * @param cachedAtMs The timestamp when the ad was cached (monotonic)
     * @param ttlMs The TTL in milliseconds
     * @param refreshIntervalMs The refresh interval in milliseconds
     * @param nowMs The current time (monotonic)
     * @return True if a pre-fetch should be triggered
     */
    fun shouldPrefetch(cachedAtMs: Long, ttlMs: Long, refreshIntervalMs: Long, nowMs: Long): Boolean {
        val remaining = remainingTTLMs(cachedAtMs, ttlMs, nowMs)
        return remaining > 0 && remaining < refreshIntervalMs
    }
    
    /**
     * Data class representing cache entry metadata with TTL.
     */
    data class CacheMetadata(
        val cachedAtMs: Long,
        val ttlMs: Long,
        val refreshIntervalMs: Long
    ) {
        val expiresAtMs: Long get() = cachedAtMs + ttlMs
        
        fun isExpired(nowMs: Long): Boolean = isExpired(cachedAtMs, ttlMs, nowMs)
        
        fun remainingMs(nowMs: Long): Long = remainingTTLMs(cachedAtMs, ttlMs, nowMs)
        
        fun shouldPrefetch(nowMs: Long): Boolean = 
            AdCacheTTL.shouldPrefetch(cachedAtMs, ttlMs, refreshIntervalMs, nowMs)
    }
}
