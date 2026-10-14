/**
 * NoFillTracker - Tracks no-fill events for analytics and pattern detection
 *
 * Features:
 * - Event recording with rich metadata
 * - Hourly and daily aggregation
 * - Pattern detection (consecutive failures, elevated rates)
 * - Statistics and breakdown by source, placement, reason
 */

/**
 * Reasons for ad no-fill
 */
export enum NoFillReason {
  TIMEOUT = 'timeout',
  NO_INVENTORY = 'no_inventory',
  NETWORK_ERROR = 'network_error',
  POLICY_VIOLATION = 'policy_violation',
  FREQUENCY_CAP = 'frequency_cap',
  GEOGRAPHIC_RESTRICTION = 'geographic_restriction',
  BUDGET_EXHAUSTED = 'budget_exhausted',
  MALFORMED_RESPONSE = 'malformed_response',
  SERVER_ERROR = 'server_error',
  UNKNOWN = 'unknown',
}

/**
 * A single no-fill event
 */
export interface NoFillEvent {
  sourceId: string;
  placementId: string;
  reason: NoFillReason;
  timestamp: number;
  latencyMs: number;
  metadata: Record<string, string>;
}

/**
 * Statistics for no-fill events
 */
export interface NoFillStats {
  totalNoFills: number;
  noFillRate: number; // Per minute
  averageLatencyMs: number;
  topReasons: Map<NoFillReason, number>;
  topSources: Map<string, number>;
  topPlacements: Map<string, number>;
  hourlyBreakdown: Map<number, number>;
  dailyBreakdown: Map<number, number>;
}

/**
 * Pattern type
 */
export enum PatternType {
  ELEVATED_RATE = 'elevated_rate',
  SOURCE_SPECIFIC = 'source_specific',
  PLACEMENT_SPECIFIC = 'placement_specific',
  CONSECUTIVE_FAILURES = 'consecutive_failures',
}

/**
 * Pattern severity
 */
export enum PatternSeverity {
  LOW = 'low',
  MEDIUM = 'medium',
  HIGH = 'high',
  CRITICAL = 'critical',
}

/**
 * Detected no-fill pattern
 */
export interface NoFillPattern {
  type: PatternType;
  severity: PatternSeverity;
  description: string;
  detectedAt: number;
  affectedSourceId?: string;
  affectedPlacementId?: string;
}

/**
 * Configuration for NoFillTracker
 */
export interface NoFillTrackerConfig {
  maxEventsRetained: number;
  maxRetentionHours: number;
  elevatedRateThreshold: number;
  patternDetectionEnabled: boolean;
  consecutiveFailureThreshold: number;
}

/**
 * Default configuration
 */
export const DEFAULT_CONFIG: NoFillTrackerConfig = {
  maxEventsRetained: 10000,
  maxRetentionHours: 24,
  elevatedRateThreshold: 0.5,
  patternDetectionEnabled: true,
  consecutiveFailureThreshold: 5,
};

/**
 * Pattern listener type
 */
export type NoFillPatternListener = (pattern: NoFillPattern) => void;

/**
 * NoFillTracker class for tracking and analyzing no-fill events
 */
export class NoFillTracker {
  private static instance: NoFillTracker | null = null;

  private config: NoFillTrackerConfig;
  private events: NoFillEvent[] = [];
  private totalNoFills = 0;
  private totalLatencyMs = 0;

  private hourlyNoFills: Map<number, number> = new Map();
  private dailyNoFills: Map<number, number> = new Map();
  private noFillsBySource: Map<string, number> = new Map();
  private noFillsByPlacement: Map<string, number> = new Map();
  private noFillsByReason: Map<NoFillReason, number> = new Map();
  private consecutiveNoFillsBySource: Map<string, number> = new Map();
  private detectedPatterns: NoFillPattern[] = [];
  private patternListeners: NoFillPatternListener[] = [];

  constructor(config: Partial<NoFillTrackerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Get singleton instance
   */
  static getInstance(): NoFillTracker {
    if (!NoFillTracker.instance) {
      NoFillTracker.instance = new NoFillTracker();
    }
    return NoFillTracker.instance;
  }

  /**
   * Reset singleton instance (for testing)
   */
  static resetInstance(): void {
    NoFillTracker.instance = null;
  }

  /**
   * Record a no-fill event
   */
  recordNoFill(
    sourceId: string,
    placementId: string,
    reason: NoFillReason,
    latencyMs: number = 0,
    metadata: Record<string, string> = {}
  ): void {
    const event: NoFillEvent = {
      sourceId,
      placementId,
      reason,
      timestamp: Date.now(),
      latencyMs,
      metadata,
    };

    this.events.push(event);
    this.totalNoFills++;
    this.totalLatencyMs += latencyMs;

    // Update source counter
    this.noFillsBySource.set(
      sourceId,
      (this.noFillsBySource.get(sourceId) || 0) + 1
    );

    // Update placement counter
    this.noFillsByPlacement.set(
      placementId,
      (this.noFillsByPlacement.get(placementId) || 0) + 1
    );

    // Update reason counter
    this.noFillsByReason.set(
      reason,
      (this.noFillsByReason.get(reason) || 0) + 1
    );

    // Update hourly breakdown
    const date = new Date(event.timestamp);
    const hour = date.getHours();
    this.hourlyNoFills.set(hour, (this.hourlyNoFills.get(hour) || 0) + 1);

    // Update daily breakdown
    const dayOfWeek = date.getDay();
    this.dailyNoFills.set(
      dayOfWeek,
      (this.dailyNoFills.get(dayOfWeek) || 0) + 1
    );

    // Update consecutive failures
    this.consecutiveNoFillsBySource.set(
      sourceId,
      (this.consecutiveNoFillsBySource.get(sourceId) || 0) + 1
    );

    // Cleanup old events
    this.cleanupIfNeeded();

    // Detect patterns
    if (this.config.patternDetectionEnabled) {
      this.detectPatternsForEvent(event);
    }
  }

  /**
   * Record a successful fill (resets consecutive failure counter)
   */
  recordFill(sourceId: string): void {
    this.consecutiveNoFillsBySource.set(sourceId, 0);
  }

  /**
   * Get current statistics
   */
  getStats(): NoFillStats {
    const avgLatency =
      this.totalNoFills > 0 ? this.totalLatencyMs / this.totalNoFills : 0;

    // Calculate rate per minute (based on last hour)
    const oneHourAgo = Date.now() - 3600000;
    const recentEvents = this.events.filter((e) => e.timestamp >= oneHourAgo);
    const ratePerMinute = recentEvents.length / 60;

    // Get top reasons (sorted by count)
    const topReasons = new Map(
      [...this.noFillsByReason.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 5)
    );

    // Get top sources
    const topSources = new Map(
      [...this.noFillsBySource.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 5)
    );

    // Get top placements
    const topPlacements = new Map(
      [...this.noFillsByPlacement.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 5)
    );

    return {
      totalNoFills: this.totalNoFills,
      noFillRate: ratePerMinute,
      averageLatencyMs: avgLatency,
      topReasons,
      topSources,
      topPlacements,
      hourlyBreakdown: new Map(this.hourlyNoFills),
      dailyBreakdown: new Map(this.dailyNoFills),
    };
  }

  /**
   * Get hourly breakdown
   */
  getHourlyBreakdown(): Map<number, number> {
    return new Map(this.hourlyNoFills);
  }

  /**
   * Get daily breakdown
   */
  getDailyBreakdown(): Map<number, number> {
    return new Map(this.dailyNoFills);
  }

  /**
   * Get no-fills by source
   */
  getNoFillsBySource(): Map<string, number> {
    return new Map(this.noFillsBySource);
  }

  /**
   * Get no-fills by placement
   */
  getNoFillsByPlacement(): Map<string, number> {
    return new Map(this.noFillsByPlacement);
  }

  /**
   * Get no-fills by reason
   */
  getNoFillsByReason(): Map<NoFillReason, number> {
    return new Map(this.noFillsByReason);
  }

  /**
   * Get events in a time range
   */
  getEvents(from: number, to: number): NoFillEvent[] {
    return this.events.filter((e) => e.timestamp >= from && e.timestamp <= to);
  }

  /**
   * Get recent events
   */
  getRecentEvents(count: number = 100): NoFillEvent[] {
    return this.events.slice(-count);
  }

  /**
   * Get detected patterns
   */
  getDetectedPatterns(): NoFillPattern[] {
    return [...this.detectedPatterns];
  }

  /**
   * Add pattern listener
   */
  addPatternListener(listener: NoFillPatternListener): void {
    this.patternListeners.push(listener);
  }

  /**
   * Remove pattern listener
   */
  removePatternListener(listener: NoFillPatternListener): void {
    const index = this.patternListeners.indexOf(listener);
    if (index !== -1) {
      this.patternListeners.splice(index, 1);
    }
  }

  /**
   * Clear all data
   */
  clear(): void {
    this.events = [];
    this.totalNoFills = 0;
    this.totalLatencyMs = 0;
    this.hourlyNoFills.clear();
    this.dailyNoFills.clear();
    this.noFillsBySource.clear();
    this.noFillsByPlacement.clear();
    this.noFillsByReason.clear();
    this.consecutiveNoFillsBySource.clear();
    this.detectedPatterns = [];
  }

  /**
   * Update configuration
   */
  updateConfiguration(config: Partial<NoFillTrackerConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration
   */
  getConfiguration(): NoFillTrackerConfig {
    return { ...this.config };
  }

  private cleanupIfNeeded(): void {
    // Remove events exceeding retention limit
    const cutoff = Date.now() - this.config.maxRetentionHours * 3600000;
    this.events = this.events.filter((e) => e.timestamp >= cutoff);

    // Trim to max events
    if (this.events.length > this.config.maxEventsRetained) {
      const excess = this.events.length - this.config.maxEventsRetained;
      this.events.splice(0, excess);
    }
  }

  private detectPatternsForEvent(event: NoFillEvent): void {
    // Check for consecutive failures
    const consecutive =
      this.consecutiveNoFillsBySource.get(event.sourceId) || 0;
    if (consecutive >= this.config.consecutiveFailureThreshold) {
      const severity =
        consecutive >= 10
          ? PatternSeverity.HIGH
          : consecutive >= 5
            ? PatternSeverity.MEDIUM
            : PatternSeverity.LOW;

      const pattern: NoFillPattern = {
        type: PatternType.CONSECUTIVE_FAILURES,
        severity,
        description: `Source ${event.sourceId} has ${consecutive} consecutive no-fills`,
        detectedAt: Date.now(),
        affectedSourceId: event.sourceId,
      };
      this.addPattern(pattern);
    }

    // Check for elevated rate on this source
    const sourceTotal = this.noFillsBySource.get(event.sourceId) || 0;
    const totalAttempts = Math.max(1, this.totalNoFills);
    const sourceRate = sourceTotal / totalAttempts;

    if (sourceRate > this.config.elevatedRateThreshold && sourceTotal > 10) {
      const severity =
        sourceRate > 0.8 ? PatternSeverity.CRITICAL : PatternSeverity.MEDIUM;

      const pattern: NoFillPattern = {
        type: PatternType.SOURCE_SPECIFIC,
        severity,
        description: `Source ${event.sourceId} has ${Math.round(sourceRate * 100)}% no-fill rate`,
        detectedAt: Date.now(),
        affectedSourceId: event.sourceId,
      };
      this.addPattern(pattern);
    }

    // Check for placement-specific issues
    const placementTotal = this.noFillsByPlacement.get(event.placementId) || 0;
    if (placementTotal > 20) {
      const placementRate = placementTotal / Math.max(1, this.totalNoFills);
      if (placementRate > 0.3) {
        const pattern: NoFillPattern = {
          type: PatternType.PLACEMENT_SPECIFIC,
          severity: PatternSeverity.MEDIUM,
          description: `Placement ${event.placementId} accounts for ${Math.round(placementRate * 100)}% of no-fills`,
          detectedAt: Date.now(),
          affectedPlacementId: event.placementId,
        };
        this.addPattern(pattern);
      }
    }
  }

  private addPattern(pattern: NoFillPattern): void {
    // Avoid duplicate patterns within 5 minutes
    const recentCutoff = Date.now() - 300000;
    const isDuplicate = this.detectedPatterns.some(
      (existing) =>
        existing.type === pattern.type &&
        existing.affectedSourceId === pattern.affectedSourceId &&
        existing.affectedPlacementId === pattern.affectedPlacementId &&
        existing.detectedAt > recentCutoff
    );

    if (!isDuplicate) {
      this.detectedPatterns.push(pattern);

      // Trim old patterns
      const oneDayAgo = Date.now() - 86400000;
      this.detectedPatterns = this.detectedPatterns.filter(
        (p) => p.detectedAt >= oneDayAgo
      );

      // Notify listeners
      for (const listener of this.patternListeners) {
        try {
          listener(pattern);
        } catch {
          // Ignore listener errors
        }
      }
    }
  }
}

/**
 * Analyzer for no-fill patterns and recommendations
 */
export class NoFillAnalyzer {
  private tracker: NoFillTracker;

  constructor(tracker?: NoFillTracker) {
    this.tracker = tracker || NoFillTracker.getInstance();
  }

  /**
   * Analyze current no-fill patterns
   */
  analyze(): {
    summary: string;
    recommendations: string[];
    healthScore: number;
    problematicSources: string[];
    problematicPlacements: string[];
  } {
    const stats = this.tracker.getStats();
    const patterns = this.tracker.getDetectedPatterns();

    const recommendations: string[] = [];
    const problematicSources: string[] = [];
    const problematicPlacements: string[] = [];

    // Analyze top sources
    for (const [source, count] of stats.topSources) {
      const percentage = (count / Math.max(1, stats.totalNoFills)) * 100;
      if (percentage > 30) {
        problematicSources.push(source);
        recommendations.push(
          `Consider deprioritizing source '${source}' (accounts for ${Math.round(percentage)}% of no-fills)`
        );
      }
    }

    // Analyze top placements
    for (const [placement, count] of stats.topPlacements) {
      const percentage = (count / Math.max(1, stats.totalNoFills)) * 100;
      if (percentage > 30) {
        problematicPlacements.push(placement);
        recommendations.push(
          `Review placement '${placement}' configuration (accounts for ${Math.round(percentage)}% of no-fills)`
        );
      }
    }

    // Analyze reasons
    let topReason: [NoFillReason, number] | undefined;
    let maxCount = 0;
    for (const [reason, count] of stats.topReasons) {
      if (count > maxCount) {
        maxCount = count;
        topReason = [reason, count];
      }
    }

    if (topReason) {
      const percentage =
        (topReason[1] / Math.max(1, stats.totalNoFills)) * 100;
      if (percentage > 40) {
        switch (topReason[0]) {
          case NoFillReason.TIMEOUT:
            recommendations.push(
              'Consider increasing timeout thresholds or improving network conditions'
            );
            break;
          case NoFillReason.NO_INVENTORY:
            recommendations.push(
              'Explore additional demand sources to improve fill rate'
            );
            break;
          case NoFillReason.NETWORK_ERROR:
            recommendations.push(
              'Review network connectivity and retry strategies'
            );
            break;
          case NoFillReason.FREQUENCY_CAP:
            recommendations.push(
              'Review frequency cap settings - may be too restrictive'
            );
            break;
          default:
            recommendations.push(
              `Investigate high occurrence of '${topReason[0]}' errors`
            );
        }
      }
    }

    // Calculate health score
    let healthScore = 100;

    // Deduct for high no-fill rate
    if (stats.noFillRate > 10) {
      healthScore -= Math.min(50, stats.noFillRate * 2);
    }

    // Deduct for detected patterns
    const criticalPatterns = patterns.filter(
      (p) => p.severity === PatternSeverity.CRITICAL
    ).length;
    const highPatterns = patterns.filter(
      (p) => p.severity === PatternSeverity.HIGH
    ).length;
    healthScore -= criticalPatterns * 15;
    healthScore -= highPatterns * 10;
    healthScore = Math.max(0, Math.min(100, healthScore));

    // Generate summary
    let summary: string;
    if (healthScore >= 80) {
      summary = `No-fill tracking is healthy with ${stats.totalNoFills} events recorded.`;
    } else if (healthScore >= 50) {
      summary = `Moderate no-fill issues detected. ${recommendations.length} recommendations available.`;
    } else {
      summary =
        'Critical no-fill issues detected! Immediate attention recommended.';
    }

    return {
      summary,
      recommendations,
      healthScore,
      problematicSources,
      problematicPlacements,
    };
  }
}
