/**
 * @file Usage Meter
 * @description Track and report billable usage events for ad SDK
 * @module @rivalapex/web-sdk
 */

/**
 * Types of billable usage events
 */
export enum UsageEventType {
  AD_REQUEST = 'ad_request',
  AD_IMPRESSION = 'ad_impression',
  AD_CLICK = 'ad_click',
  AD_VIDEO_START = 'ad_video_start',
  AD_VIDEO_COMPLETE = 'ad_video_complete',
  AD_REVENUE = 'ad_revenue',
  API_CALL = 'api_call',
  CACHE_HIT = 'cache_hit',
  CACHE_MISS = 'cache_miss',
  ERROR = 'error',
}

/**
 * A single usage event
 */
export interface UsageEvent {
  type: UsageEventType;
  timestamp: number;
  placementId?: string;
  adapterId?: string;
  adFormat?: string;
  revenueAmount?: number;
  metadata?: Record<string, unknown>;
}

/**
 * Aggregated usage metrics
 */
export interface UsageMetrics {
  adRequests: number;
  adImpressions: number;
  adClicks: number;
  videoStarts: number;
  videoCompletes: number;
  totalRevenue: number;
  apiCalls: number;
  cacheHits: number;
  cacheMisses: number;
  errors: number;
  periodStart: number;
  periodEnd: number;
}

/**
 * Breakdown by dimension
 */
export interface UsageBreakdown {
  byPlacement: Map<string, UsageMetrics>;
  byAdapter: Map<string, UsageMetrics>;
  byAdFormat: Map<string, UsageMetrics>;
}

/**
 * Metering configuration
 */
export interface MeteringConfig {
  flushIntervalMs?: number;
  maxEventsBeforeFlush?: number;
  enableLocalStorage?: boolean;
  enableRemoteReporting?: boolean;
  samplingRate?: number;
  reportEndpoint?: string;
  apiKey?: string;
}

/**
 * Reporter interface for sending metrics
 */
export interface MeteringReporter {
  report(metrics: UsageMetrics, breakdown: UsageBreakdown): Promise<boolean>;
}

/**
 * Dimension-specific metrics
 */
class DimensionMetrics {
  adRequests = 0;
  adImpressions = 0;
  adClicks = 0;
  videoStarts = 0;
  videoCompletes = 0;
  totalRevenue = 0;
  apiCalls = 0;
  cacheHits = 0;
  cacheMisses = 0;
  errors = 0;

  toMetrics(periodStart: number, periodEnd: number): UsageMetrics {
    return {
      adRequests: this.adRequests,
      adImpressions: this.adImpressions,
      adClicks: this.adClicks,
      videoStarts: this.videoStarts,
      videoCompletes: this.videoCompletes,
      totalRevenue: this.totalRevenue,
      apiCalls: this.apiCalls,
      cacheHits: this.cacheHits,
      cacheMisses: this.cacheMisses,
      errors: this.errors,
      periodStart,
      periodEnd,
    };
  }

  reset(): void {
    this.adRequests = 0;
    this.adImpressions = 0;
    this.adClicks = 0;
    this.videoStarts = 0;
    this.videoCompletes = 0;
    this.totalRevenue = 0;
    this.apiCalls = 0;
    this.cacheHits = 0;
    this.cacheMisses = 0;
    this.errors = 0;
  }
}

/**
 * Default configuration
 */
const DEFAULT_CONFIG: Required<MeteringConfig> = {
  flushIntervalMs: 60000,
  maxEventsBeforeFlush: 1000,
  enableLocalStorage: true,
  enableRemoteReporting: true,
  samplingRate: 1.0,
  reportEndpoint: '/api/v1/metrics/usage',
};

/**
 * Usage meter for tracking billable events
 */
export class UsageMeter {
  private config: Required<MeteringConfig>;
  private reporter?: MeteringReporter;

  // Global metrics
  private globalMetrics = new DimensionMetrics();

  // Dimensional breakdowns
  private placementMetrics = new Map<string, DimensionMetrics>();
  private adapterMetrics = new Map<string, DimensionMetrics>();
  private formatMetrics = new Map<string, DimensionMetrics>();

  // Pending events
  private pendingEvents: UsageEvent[] = [];

  // Period tracking
  private periodStart: number = Date.now();

  // Flush timer
  private flushTimer: ReturnType<typeof setInterval> | null = null;
  private isRunning = false;

  constructor(config: MeteringConfig = {}, reporter?: MeteringReporter) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.reporter = reporter;
  }

  /**
   * Start the metering service
   */
  start(): void {
    if (this.isRunning) return;
    this.isRunning = true;
    this.periodStart = Date.now();

    this.flushTimer = setInterval(() => {
      this.flush().catch(() => {});
    }, this.config.flushIntervalMs);
  }

  /**
   * Stop the metering service
   */
  stop(): void {
    this.isRunning = false;
    if (this.flushTimer) {
      clearInterval(this.flushTimer);
      this.flushTimer = null;
    }
  }

  /**
   * Record a usage event
   */
  record(event: UsageEvent): void {
    // Apply sampling
    if (this.config.samplingRate < 1.0 && Math.random() > this.config.samplingRate) {
      return;
    }

    // Update global metrics
    this.updateMetrics(this.globalMetrics, event);

    // Update dimensional breakdowns
    if (event.placementId) {
      const metrics = this.getOrCreateMetrics(this.placementMetrics, event.placementId);
      this.updateMetrics(metrics, event);
    }

    if (event.adapterId) {
      const metrics = this.getOrCreateMetrics(this.adapterMetrics, event.adapterId);
      this.updateMetrics(metrics, event);
    }

    if (event.adFormat) {
      const metrics = this.getOrCreateMetrics(this.formatMetrics, event.adFormat);
      this.updateMetrics(metrics, event);
    }

    // Queue event for storage
    if (this.config.enableLocalStorage) {
      this.pendingEvents.push(event);
      if (this.pendingEvents.length >= this.config.maxEventsBeforeFlush) {
        this.flush().catch(() => {});
      }
    }
  }

  private getOrCreateMetrics(map: Map<string, DimensionMetrics>, key: string): DimensionMetrics {
    let metrics = map.get(key);
    if (!metrics) {
      metrics = new DimensionMetrics();
      map.set(key, metrics);
    }
    return metrics;
  }

  private updateMetrics(metrics: DimensionMetrics, event: UsageEvent): void {
    switch (event.type) {
      case UsageEventType.AD_REQUEST:
        metrics.adRequests++;
        break;
      case UsageEventType.AD_IMPRESSION:
        metrics.adImpressions++;
        break;
      case UsageEventType.AD_CLICK:
        metrics.adClicks++;
        break;
      case UsageEventType.AD_VIDEO_START:
        metrics.videoStarts++;
        break;
      case UsageEventType.AD_VIDEO_COMPLETE:
        metrics.videoCompletes++;
        break;
      case UsageEventType.AD_REVENUE:
        if (event.revenueAmount != null) {
          metrics.totalRevenue += event.revenueAmount;
        }
        break;
      case UsageEventType.API_CALL:
        metrics.apiCalls++;
        break;
      case UsageEventType.CACHE_HIT:
        metrics.cacheHits++;
        break;
      case UsageEventType.CACHE_MISS:
        metrics.cacheMisses++;
        break;
      case UsageEventType.ERROR:
        metrics.errors++;
        break;
    }
  }

  // Convenience methods for recording events
  recordRequest(placementId: string, adapterId?: string, adFormat?: string): void {
    this.record({
      type: UsageEventType.AD_REQUEST,
      timestamp: Date.now(),
      placementId,
      adapterId,
      adFormat,
    });
  }

  recordImpression(placementId: string, adapterId: string, adFormat?: string): void {
    this.record({
      type: UsageEventType.AD_IMPRESSION,
      timestamp: Date.now(),
      placementId,
      adapterId,
      adFormat,
    });
  }

  recordClick(placementId: string, adapterId: string, adFormat?: string): void {
    this.record({
      type: UsageEventType.AD_CLICK,
      timestamp: Date.now(),
      placementId,
      adapterId,
      adFormat,
    });
  }

  recordRevenue(placementId: string, adapterId: string, amount: number): void {
    this.record({
      type: UsageEventType.AD_REVENUE,
      timestamp: Date.now(),
      placementId,
      adapterId,
      revenueAmount: amount,
    });
  }

  recordVideoStart(placementId: string, adapterId: string): void {
    this.record({
      type: UsageEventType.AD_VIDEO_START,
      timestamp: Date.now(),
      placementId,
      adapterId,
      adFormat: 'video',
    });
  }

  recordVideoComplete(placementId: string, adapterId: string): void {
    this.record({
      type: UsageEventType.AD_VIDEO_COMPLETE,
      timestamp: Date.now(),
      placementId,
      adapterId,
      adFormat: 'video',
    });
  }

  recordCacheHit(placementId?: string): void {
    this.record({
      type: UsageEventType.CACHE_HIT,
      timestamp: Date.now(),
      placementId,
    });
  }

  recordCacheMiss(placementId?: string): void {
    this.record({
      type: UsageEventType.CACHE_MISS,
      timestamp: Date.now(),
      placementId,
    });
  }

  recordError(placementId?: string, adapterId?: string): void {
    this.record({
      type: UsageEventType.ERROR,
      timestamp: Date.now(),
      placementId,
      adapterId,
    });
  }

  /**
   * Get current metrics snapshot
   */
  getMetrics(): UsageMetrics {
    return this.globalMetrics.toMetrics(this.periodStart, Date.now());
  }

  /**
   * Get breakdown by dimensions
   */
  getBreakdown(): UsageBreakdown {
    const now = Date.now();
    return {
      byPlacement: new Map(
        Array.from(this.placementMetrics.entries()).map(([k, v]) => [
          k,
          v.toMetrics(this.periodStart, now),
        ])
      ),
      byAdapter: new Map(
        Array.from(this.adapterMetrics.entries()).map(([k, v]) => [
          k,
          v.toMetrics(this.periodStart, now),
        ])
      ),
      byAdFormat: new Map(
        Array.from(this.formatMetrics.entries()).map(([k, v]) => [
          k,
          v.toMetrics(this.periodStart, now),
        ])
      ),
    };
  }

  /**
   * Get click-through rate
   */
  getCTR(): number {
    const metrics = this.getMetrics();
    if (metrics.adImpressions === 0) return 0;
    return metrics.adClicks / metrics.adImpressions;
  }

  /**
   * Get fill rate
   */
  getFillRate(): number {
    const metrics = this.getMetrics();
    if (metrics.adRequests === 0) return 0;
    return metrics.adImpressions / metrics.adRequests;
  }

  /**
   * Get video completion rate
   */
  getVideoCompletionRate(): number {
    const metrics = this.getMetrics();
    if (metrics.videoStarts === 0) return 0;
    return metrics.videoCompletes / metrics.videoStarts;
  }

  /**
   * Get cache hit rate
   */
  getCacheHitRate(): number {
    const metrics = this.getMetrics();
    const total = metrics.cacheHits + metrics.cacheMisses;
    if (total === 0) return 0;
    return metrics.cacheHits / total;
  }

  /**
   * Get effective CPM
   */
  getEffectiveCPM(): number {
    const metrics = this.getMetrics();
    if (metrics.adImpressions === 0) return 0;
    return (metrics.totalRevenue / metrics.adImpressions) * 1000;
  }

  /**
   * Flush metrics to reporter
   */
  async flush(): Promise<boolean> {
    if (!this.config.enableRemoteReporting) return true;

    const metrics = this.getMetrics();
    const breakdown = this.getBreakdown();

    try {
      if (this.reporter) {
        const success = await this.reporter.report(metrics, breakdown);
        if (success) {
          this.reset();
        }
        return success;
      }

      // Default fetch-based reporting
      const response = await fetch(this.config.reportEndpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(this.config.apiKey ? { 'X-Api-Key': this.config.apiKey } : {}),
        },
        body: JSON.stringify({
          metrics,
          breakdown: {
            byPlacement: Object.fromEntries(breakdown.byPlacement),
            byAdapter: Object.fromEntries(breakdown.byAdapter),
            byAdFormat: Object.fromEntries(breakdown.byAdFormat),
          },
        }),
      });

      if (response.ok) {
        this.reset();
        return true;
      }
      return false;
    } catch {
      return false;
    }
  }

  /**
   * Reset all counters
   */
  reset(): void {
    this.globalMetrics.reset();
    this.placementMetrics.forEach(m => m.reset());
    this.adapterMetrics.forEach(m => m.reset());
    this.formatMetrics.forEach(m => m.reset());
    this.pendingEvents = [];
    this.periodStart = Date.now();
  }

  /**
   * Export metrics as JSON
   */
  exportAsJSON(): string {
    const metrics = this.getMetrics();
    const breakdown = this.getBreakdown();

    return JSON.stringify(
      {
        metrics,
        computed: {
          ctr: this.getCTR(),
          fillRate: this.getFillRate(),
          videoCompletionRate: this.getVideoCompletionRate(),
          cacheHitRate: this.getCacheHitRate(),
          effectiveCPM: this.getEffectiveCPM(),
        },
        breakdown: {
          placementCount: breakdown.byPlacement.size,
          adapterCount: breakdown.byAdapter.size,
          formatCount: breakdown.byAdFormat.size,
        },
      },
      null,
      2
    );
  }

  /**
   * Get pending events count
   */
  getPendingEventsCount(): number {
    return this.pendingEvents.length;
  }
}

/**
 * Builder for UsageMeter
 */
export class UsageMeterBuilder {
  private config: MeteringConfig = {};
  private reporter?: MeteringReporter;

  withConfig(config: MeteringConfig): this {
    this.config = { ...this.config, ...config };
    return this;
  }

  withReporter(reporter: MeteringReporter): this {
    this.reporter = reporter;
    return this;
  }

  withFlushInterval(ms: number): this {
    this.config.flushIntervalMs = ms;
    return this;
  }

  withSamplingRate(rate: number): this {
    this.config.samplingRate = Math.max(0, Math.min(1, rate));
    return this;
  }

  withReportEndpoint(endpoint: string): this {
    this.config.reportEndpoint = endpoint;
    return this;
  }

  withApiKey(apiKey: string): this {
    this.config.apiKey = apiKey;
    return this;
  }

  enableLocalStorage(enabled: boolean): this {
    this.config.enableLocalStorage = enabled;
    return this;
  }

  enableRemoteReporting(enabled: boolean): this {
    this.config.enableRemoteReporting = enabled;
    return this;
  }

  build(): UsageMeter {
    return new UsageMeter(this.config, this.reporter);
  }
}
