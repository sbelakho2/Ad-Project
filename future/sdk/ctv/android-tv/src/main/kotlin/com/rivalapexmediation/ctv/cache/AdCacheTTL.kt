package com.rivalapexmediation.ctv.cache

import kotlin.math.max
import kotlin.math.min

/**
 * AdCacheTTL - Computes ad cache TTL based on refresh interval for Android TV.
 * 
 * The TTL is calculated as 2× the refresh interval, capped at 60 minutes.
 * For CTV environments, this is particularly important as video ads may
 * have longer durations between refresh cycles.
 */
object AdCacheTTL {
    
    /** Maximum TTL in milliseconds (60 minutes) */
    const val MAX_TTL_MS: Long = 60 * 60 * 1000L
    
    /** Maximum TTL in seconds (60 minutes) */
    const val MAX_TTL_SECONDS: Long = 60 * 60L
    
    /** Default refresh interval in milliseconds for CTV (longer than mobile) */
    const val DEFAULT_REFRESH_INTERVAL_MS: Long = 60 * 1000L
    
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
     * Calculates the effective TTL for a video ad pod.
     * For video pods, we consider the total pod duration.
     * 
     * @param refreshIntervalMs The ad refresh interval in milliseconds
     * @param podDurationMs The total duration of the ad pod (optional)
     * @param serverTtlSeconds The TTL provided by the server (optional)
     * @return The effective TTL in milliseconds
     */
    fun calculateEffectiveTTLMs(
        refreshIntervalMs: Long,
        podDurationMs: Long? = null,
        serverTtlSeconds: Int? = null
    ): Long {
        var baseTtl = calculateTTLMs(refreshIntervalMs)
        
        // For video pods, ensure TTL covers the pod duration plus buffer
        podDurationMs?.let { podDuration ->
            if (podDuration > 0) {
                val podBasedTtl = podDuration + 30_000L // 30-second buffer
                baseTtl = max(baseTtl, podBasedTtl)
            }
        }
        
        // Respect server TTL if provided
        serverTtlSeconds?.let { serverTtl ->
            if (serverTtl > 0) {
                val serverTtlMs = serverTtl.toLong() * 1000L
                baseTtl = min(baseTtl, serverTtlMs)
            }
        }
        
        return min(baseTtl, MAX_TTL_MS)
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
     * For CTV, we prefetch earlier to ensure seamless video playback.
     * 
     * @param cachedAtMs The timestamp when the ad was cached (monotonic)
     * @param ttlMs The TTL in milliseconds
     * @param refreshIntervalMs The refresh interval in milliseconds
     * @param nowMs The current time (monotonic)
     * @return True if a pre-fetch should be triggered
     */
    fun shouldPrefetch(cachedAtMs: Long, ttlMs: Long, refreshIntervalMs: Long, nowMs: Long): Boolean {
        val remaining = remainingTTLMs(cachedAtMs, ttlMs, nowMs)
        // For CTV, prefetch when remaining TTL is less than 1.5× refresh interval
        val prefetchThreshold = (refreshIntervalMs * 1.5).toLong()
        return remaining > 0 && remaining < prefetchThreshold
    }
    
    /**
     * Data class representing cache entry metadata with TTL for CTV.
     */
    data class CacheMetadata(
        val cachedAtMs: Long,
        val ttlMs: Long,
        val refreshIntervalMs: Long,
        val podDurationMs: Long? = null
    ) {
        val expiresAtMs: Long get() = cachedAtMs + ttlMs
        
        fun isExpired(nowMs: Long): Boolean = AdCacheTTL.isExpired(cachedAtMs, ttlMs, nowMs)
        
        fun remainingMs(nowMs: Long): Long = remainingTTLMs(cachedAtMs, ttlMs, nowMs)
        
        fun shouldPrefetch(nowMs: Long): Boolean = 
            AdCacheTTL.shouldPrefetch(cachedAtMs, ttlMs, refreshIntervalMs, nowMs)
    }
}
