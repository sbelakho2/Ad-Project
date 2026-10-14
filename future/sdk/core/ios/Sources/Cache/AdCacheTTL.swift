import Foundation

/// AdCacheTTL - Computes ad cache TTL based on refresh interval.
///
/// The TTL is calculated as 2× the refresh interval, capped at 60 minutes.
/// This ensures ads remain cached long enough for the next refresh cycle
/// while preventing stale ads from lingering too long.
public enum AdCacheTTL {
    
    /// Maximum TTL in seconds (60 minutes)
    public static let maxTTLSeconds: TimeInterval = 60 * 60
    
    /// Maximum TTL in milliseconds (60 minutes)
    public static let maxTTLMs: Int64 = 60 * 60 * 1000
    
    /// Default refresh interval in seconds if none specified
    public static let defaultRefreshIntervalSeconds: TimeInterval = 30
    
    /// Minimum TTL in seconds (30 seconds)
    public static let minTTLSeconds: TimeInterval = 30
    
    /// Calculates the cache TTL based on the refresh interval.
    ///
    /// - Parameter refreshIntervalSeconds: The ad refresh interval in seconds
    /// - Returns: The TTL in seconds (2× refreshInterval, capped at 60 minutes)
    public static func calculateTTL(refreshIntervalSeconds: TimeInterval) -> TimeInterval {
        if refreshIntervalSeconds <= 0 {
            return min(defaultRefreshIntervalSeconds * 2, maxTTLSeconds)
        }
        let ttl = refreshIntervalSeconds * 2
        return max(minTTLSeconds, min(ttl, maxTTLSeconds))
    }
    
    /// Calculates the cache TTL based on the refresh interval in milliseconds.
    ///
    /// - Parameter refreshIntervalMs: The ad refresh interval in milliseconds
    /// - Returns: The TTL in milliseconds (2× refreshInterval, capped at 60 minutes)
    public static func calculateTTLMs(refreshIntervalMs: Int64) -> Int64 {
        if refreshIntervalMs <= 0 {
            let defaultMs: Int64 = Int64(defaultRefreshIntervalSeconds * 1000)
            return min(defaultMs * 2, maxTTLMs)
        }
        let ttl = refreshIntervalMs * 2
        let minTTLMs: Int64 = Int64(minTTLSeconds * 1000)
        return max(minTTLMs, min(ttl, maxTTLMs))
    }
    
    /// Calculates the effective TTL, respecting any server-provided TTL.
    ///
    /// If the server provides a TTL, we use the minimum of:
    /// - Server-provided TTL
    /// - Calculated TTL (2× refreshInterval)
    /// - Maximum TTL (60 minutes)
    ///
    /// - Parameters:
    ///   - refreshIntervalSeconds: The ad refresh interval in seconds
    ///   - serverTTLSeconds: The TTL provided by the server (optional)
    /// - Returns: The effective TTL in seconds
    public static func calculateEffectiveTTL(
        refreshIntervalSeconds: TimeInterval,
        serverTTLSeconds: TimeInterval?
    ) -> TimeInterval {
        let calculatedTTL = calculateTTL(refreshIntervalSeconds: refreshIntervalSeconds)
        
        if let serverTTL = serverTTLSeconds, serverTTL > 0 {
            return min(calculatedTTL, min(serverTTL, maxTTLSeconds))
        }
        
        return calculatedTTL
    }
    
    /// Checks if an ad has expired based on its cache timestamp and TTL.
    ///
    /// - Parameters:
    ///   - cachedAt: The timestamp when the ad was cached (monotonic)
    ///   - ttlSeconds: The TTL in seconds
    ///   - now: The current time (monotonic)
    /// - Returns: True if the ad has expired
    public static func isExpired(
        cachedAt: TimeInterval,
        ttlSeconds: TimeInterval,
        now: TimeInterval
    ) -> Bool {
        return now >= cachedAt + ttlSeconds
    }
    
    /// Gets the remaining TTL for a cached ad.
    ///
    /// - Parameters:
    ///   - cachedAt: The timestamp when the ad was cached (monotonic)
    ///   - ttlSeconds: The TTL in seconds
    ///   - now: The current time (monotonic)
    /// - Returns: The remaining TTL in seconds (0 if expired)
    public static func remainingTTL(
        cachedAt: TimeInterval,
        ttlSeconds: TimeInterval,
        now: TimeInterval
    ) -> TimeInterval {
        let expiresAt = cachedAt + ttlSeconds
        return max(0, expiresAt - now)
    }
    
    /// Determines if an ad should be pre-fetched based on remaining TTL.
    /// Pre-fetch should occur when remaining TTL is less than the refresh interval.
    ///
    /// - Parameters:
    ///   - cachedAt: The timestamp when the ad was cached (monotonic)
    ///   - ttlSeconds: The TTL in seconds
    ///   - refreshIntervalSeconds: The refresh interval in seconds
    ///   - now: The current time (monotonic)
    /// - Returns: True if a pre-fetch should be triggered
    public static func shouldPrefetch(
        cachedAt: TimeInterval,
        ttlSeconds: TimeInterval,
        refreshIntervalSeconds: TimeInterval,
        now: TimeInterval
    ) -> Bool {
        let remaining = remainingTTL(cachedAt: cachedAt, ttlSeconds: ttlSeconds, now: now)
        return remaining > 0 && remaining < refreshIntervalSeconds
    }
}

/// Metadata for a cached ad entry with TTL information.
public struct AdCacheMetadata {
    public let cachedAt: TimeInterval
    public let ttlSeconds: TimeInterval
    public let refreshIntervalSeconds: TimeInterval
    
    public init(
        cachedAt: TimeInterval,
        ttlSeconds: TimeInterval,
        refreshIntervalSeconds: TimeInterval
    ) {
        self.cachedAt = cachedAt
        self.ttlSeconds = ttlSeconds
        self.refreshIntervalSeconds = refreshIntervalSeconds
    }
    
    /// When this cache entry expires
    public var expiresAt: TimeInterval {
        return cachedAt + ttlSeconds
    }
    
    /// Checks if this entry has expired
    public func isExpired(now: TimeInterval) -> Bool {
        return AdCacheTTL.isExpired(cachedAt: cachedAt, ttlSeconds: ttlSeconds, now: now)
    }
    
    /// Gets the remaining TTL
    public func remaining(now: TimeInterval) -> TimeInterval {
        return AdCacheTTL.remainingTTL(cachedAt: cachedAt, ttlSeconds: ttlSeconds, now: now)
    }
    
    /// Checks if this ad should be pre-fetched
    public func shouldPrefetch(now: TimeInterval) -> Bool {
        return AdCacheTTL.shouldPrefetch(
            cachedAt: cachedAt,
            ttlSeconds: ttlSeconds,
            refreshIntervalSeconds: refreshIntervalSeconds,
            now: now
        )
    }
}
