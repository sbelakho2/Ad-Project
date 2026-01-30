/**
 * VAST Tracking & Beacon Manager
 *
 * Handles firing tracking beacons for VAST events:
 * - Impression tracking
 * - Progress tracking (start, quartiles, complete)
 * - Click tracking
 * - Error reporting
 * - Verification events
 * - Viewability metrics
 */

import {
  VASTTrackingEventType,
  FlattenedTracking,
  VASTViewableImpression,
  VASTVerification,
  SSAIReadyCreative,
} from './types';
import VASTXMLParser from './xmlParser';
import logger from '../../../utils/logger';

// =============================================================================
// Types
// =============================================================================

export interface TrackingContext {
  /** Session ID */
  sessionId?: string;

  /** Content URL */
  contentUrl?: string;

  /** User agent */
  userAgent?: string;

  /** Client IP */
  clientIp?: string;

  /** Custom macros to replace in URLs */
  macros?: Record<string, string>;

  /** Correlation ID for logging */
  correlationId?: string;
}

export interface TrackingResult {
  /** URL that was fired */
  url: string;

  /** Whether the beacon was successfully sent */
  success: boolean;

  /** HTTP status code (if applicable) */
  statusCode?: number;

  /** Response time in ms */
  responseTimeMs: number;

  /** Error message (if failed) */
  error?: string;

  /** Timestamp */
  timestamp: Date;
}

export interface TrackingBatchResult {
  /** Event that was tracked */
  event: string;

  /** Total URLs attempted */
  totalUrls: number;

  /** Successful fires */
  successCount: number;

  /** Failed fires */
  failureCount: number;

  /** Individual results */
  results: TrackingResult[];
}

export interface BeaconManagerConfig {
  /** Timeout for beacon requests (ms) */
  beaconTimeoutMs: number;

  /** Whether to fire beacons in parallel */
  parallelFire: boolean;

  /** Maximum concurrent beacon fires */
  maxConcurrent: number;

  /** Retry count for failed beacons */
  retryCount: number;

  /** Retry delay (ms) */
  retryDelayMs: number;

  /** Whether to use sendBeacon API (client-side) */
  useSendBeacon: boolean;

  /** Whether to log beacon fires */
  logBeacons: boolean;

  /** Custom headers for beacon requests */
  headers?: Record<string, string>;

  /** Maximum URLs to track in fired set (memory protection) */
  maxFiredUrlsTracked: number;
}

export const DEFAULT_BEACON_CONFIG: BeaconManagerConfig = {
  beaconTimeoutMs: 5000,
  parallelFire: true,
  maxConcurrent: 10,
  retryCount: 1,
  retryDelayMs: 500,
  useSendBeacon: false,
  logBeacons: true,
  maxFiredUrlsTracked: 10000, // Prevent memory leaks from unbounded URL tracking
};

// =============================================================================
// Standard VAST Macros
// =============================================================================

export const VAST_MACROS = {
  /** Random cache buster */
  CACHEBUSTING: '[CACHEBUSTING]',
  CACHEBUSTER: '[CACHEBUSTER]',

  /** Timestamp */
  TIMESTAMP: '[TIMESTAMP]',

  /** Asset URI */
  ASSETURI: '[ASSETURI]',

  /** Content playhead position */
  CONTENTPLAYHEAD: '[CONTENTPLAYHEAD]',

  /** Media file URL */
  MEDIAPLAYHEAD: '[MEDIAPLAYHEAD]',

  /** Error code */
  ERRORCODE: '[ERRORCODE]',

  /** Reason for not executing verification */
  REASON: '[REASON]',

  /** Ad duration */
  DURATION: '[DURATION]',

  /** Linear ad pod position */
  ADSERVINGID: '[ADSERVINGID]',

  /** Pod position */
  PODSEQUENCE: '[PODSEQUENCE]',

  /** Ad in pod position */
  ADPOSITION: '[ADPOSITION]',

  /** OMID partner name */
  OMIDPARTNER: '[OMIDPARTNER]',

  /** Player state */
  PLAYERSTATE: '[PLAYERSTATE]',

  /** Player size */
  PLAYERSIZE: '[PLAYERSIZE]',

  /** Ad type */
  ADTYPE: '[ADTYPE]',

  /** Domain */
  DOMAIN: '[DOMAIN]',

  /** Page URL */
  PAGEURL: '[PAGEURL]',

  /** App bundle */
  APPBUNDLE: '[APPBUNDLE]',

  /** Inventory type */
  INVENTORYTYPE: '[INVENTORYTYPE]',

  /** Ad play head */
  ADPLAYHEAD: '[ADPLAYHEAD]',

  /** Breakpoint */
  BREAKPOSITION: '[BREAKPOSITION]',

  /** Click position x */
  CLICKPOS_X: '[CLICKPOS_X]',

  /** Click position y */
  CLICKPOS_Y: '[CLICKPOS_Y]',
};

// =============================================================================
// Beacon Manager
// =============================================================================

export class BeaconManager {
  private config: BeaconManagerConfig;
  private pendingBeacons: Map<string, AbortController> = new Map();
  private firedUrls: Set<string> = new Set();
  /** URLs currently being fired - prevents duplicate concurrent requests */
  private firingUrls: Set<string> = new Set();

  constructor(config: Partial<BeaconManagerConfig> = {}) {
    this.config = { ...DEFAULT_BEACON_CONFIG, ...config };
  }

  /**
   * Fire a single tracking beacon
   */
  async fire(url: string, context?: TrackingContext): Promise<TrackingResult> {
    const startTime = Date.now();
    const expandedUrl = this.expandMacros(url, context);

    // Race condition protection: Check if already fired or currently being fired
    if (this.firedUrls.has(expandedUrl)) {
      return {
        url: expandedUrl,
        success: true, // Already fired successfully
        responseTimeMs: 0,
        timestamp: new Date(),
        error: 'Duplicate beacon skipped (already fired)',
      };
    }

    if (this.firingUrls.has(expandedUrl)) {
      return {
        url: expandedUrl,
        success: true, // Already in progress
        responseTimeMs: 0,
        timestamp: new Date(),
        error: 'Duplicate beacon skipped (in progress)',
      };
    }

    // Mark as firing to prevent concurrent duplicate requests
    this.firingUrls.add(expandedUrl);

    try {
      const controller = new AbortController();
      const beaconId = `${expandedUrl}_${startTime}`;
      this.pendingBeacons.set(beaconId, controller);

      const timeoutId = setTimeout(() => controller.abort(), this.config.beaconTimeoutMs);

      try {
        const response = await fetch(expandedUrl, {
          method: 'GET',
          headers: {
            'User-Agent': context?.userAgent || 'SSAI-BeaconManager/1.0',
            ...this.config.headers,
          },
          signal: controller.signal,
          mode: 'no-cors', // Beacons typically don't need response
          keepalive: true,
        });

        clearTimeout(timeoutId);
        this.pendingBeacons.delete(beaconId);

        // Memory protection: limit the size of firedUrls set
        if (this.firedUrls.size >= this.config.maxFiredUrlsTracked) {
          // Clear oldest half to make room (simple eviction strategy)
          const toDelete = Math.floor(this.config.maxFiredUrlsTracked / 2);
          const iterator = this.firedUrls.values();
          for (let i = 0; i < toDelete; i++) {
            const next = iterator.next();
            if (!next.done) {
              this.firedUrls.delete(next.value);
            }
          }
          logger.debug('[BeaconManager] Evicted old URLs from fired set (memory protection)', {
            evicted: toDelete,
            remaining: this.firedUrls.size,
          });
        }
        this.firedUrls.add(expandedUrl);

        // In no-cors mode, response.type === 'opaque' and status is always 0
        // If we got here without an error, the request was successfully sent
        const isOpaque = response.type === 'opaque';
        const result: TrackingResult = {
          url: expandedUrl,
          success: true,
          // For opaque responses, status is 0 but request succeeded
          // Only include statusCode if it's meaningful (non-opaque response)
          statusCode: isOpaque ? undefined : response.status,
          responseTimeMs: Date.now() - startTime,
          timestamp: new Date(),
        };

        if (this.config.logBeacons) {
          logger.debug('[BeaconManager] Beacon fired successfully', {
            url: expandedUrl,
            responseType: response.type,
            statusCode: isOpaque ? 'opaque' : response.status,
            responseTimeMs: result.responseTimeMs,
            correlationId: context?.correlationId,
          });
        }

        return result;
      } finally {
        clearTimeout(timeoutId);
        this.pendingBeacons.delete(beaconId);
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      const responseTimeMs = Date.now() - startTime;

      if (this.config.logBeacons) {
        logger.warn('[BeaconManager] Beacon fire failed', {
          url: expandedUrl,
          error: errorMessage,
          responseTimeMs,
          correlationId: context?.correlationId,
        });
      }

      return {
        url: expandedUrl,
        success: false,
        responseTimeMs,
        error: errorMessage,
        timestamp: new Date(),
      };
    } finally {
      // Always clean up firingUrls to allow retries
      this.firingUrls.delete(expandedUrl);
    }
  }

  /**
   * Fire multiple beacons for an event
   */
  async fireMany(
    urls: string[],
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    const results: TrackingResult[] = [];

    if (urls.length === 0) {
      return {
        event: 'unknown',
        totalUrls: 0,
        successCount: 0,
        failureCount: 0,
        results: [],
      };
    }

    if (this.config.parallelFire) {
      // Fire in parallel with concurrency limit
      const chunks = this.chunkArray(urls, this.config.maxConcurrent);

      for (const chunk of chunks) {
        const chunkResults = await Promise.all(
          chunk.map((url) => this.fireWithRetry(url, context))
        );
        results.push(...chunkResults);
      }
    } else {
      // Fire sequentially
      for (const url of urls) {
        const result = await this.fireWithRetry(url, context);
        results.push(result);
      }
    }

    const successCount = results.filter((r) => r.success).length;

    return {
      event: 'batch',
      totalUrls: urls.length,
      successCount,
      failureCount: urls.length - successCount,
      results,
    };
  }

  /**
   * Fire beacon with retry logic
   */
  private async fireWithRetry(
    url: string,
    context?: TrackingContext
  ): Promise<TrackingResult> {
    let lastResult = await this.fire(url, context);

    for (let attempt = 0; attempt < this.config.retryCount && !lastResult.success; attempt++) {
      await this.delay(this.config.retryDelayMs);
      lastResult = await this.fire(url, context);
    }

    return lastResult;
  }

  /**
   * Expand VAST macros in URL
   */
  private expandMacros(url: string, context?: TrackingContext): string {
    let expanded = url;

    // Built-in macros
    expanded = expanded.replace(/\[CACHEBUSTING\]|\[CACHEBUSTER\]/gi, Date.now().toString());
    expanded = expanded.replace(/\[TIMESTAMP\]/gi, new Date().toISOString());

    // Custom macros from context
    if (context?.macros) {
      for (const [key, value] of Object.entries(context.macros)) {
        const patterns = [
          new RegExp(`\\[${key}\\]`, 'gi'),
          new RegExp(`\\{${key}\\}`, 'gi'),
          new RegExp(`%5B${key}%5D`, 'gi'), // URL-encoded
        ];

        for (const pattern of patterns) {
          expanded = expanded.replace(pattern, encodeURIComponent(value));
        }
      }
    }

    return expanded;
  }

  /**
   * Cancel all pending beacons
   */
  cancelAll(): void {
    for (const [id, controller] of this.pendingBeacons) {
      controller.abort();
      this.pendingBeacons.delete(id);
    }
  }

  /**
   * Check if a URL has been fired
   */
  hasBeenFired(url: string): boolean {
    return this.firedUrls.has(url);
  }

  /**
   * Check if a beacon is currently being fired
   */
  isBeingFired(url: string): boolean {
    return this.firingUrls.has(url);
  }

  /**
   * Clear fired URLs tracking
   */
  clearFiredUrls(): void {
    this.firedUrls.clear();
    this.firingUrls.clear();
  }

  private chunkArray<T>(array: T[], size: number): T[][] {
    const chunks: T[][] = [];
    for (let i = 0; i < array.length; i += size) {
      chunks.push(array.slice(i, i + size));
    }
    return chunks;
  }

  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
}

// =============================================================================
// Tracking Event Manager
// =============================================================================

/** Maximum number of creative tracking states to keep (memory protection) */
const MAX_TRACKING_STATES = 1000;

/**
 * High-level tracking manager for VAST events
 */
export class VASTTrackingManager {
  private beaconManager: BeaconManager;
  private trackingState: Map<string, Set<VASTTrackingEventType>> = new Map();
  /** Order of creative IDs for LRU eviction */
  private trackingStateOrder: string[] = [];

  constructor(beaconConfig?: Partial<BeaconManagerConfig>) {
    this.beaconManager = new BeaconManager(beaconConfig);
  }

  /**
   * Fire impressions for an ad
   */
  async fireImpressions(
    tracking: FlattenedTracking,
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    return this.beaconManager.fireMany(tracking.impressions, context);
  }

  /**
   * Fire a tracking event
   */
  async fireEvent(
    tracking: FlattenedTracking,
    event: VASTTrackingEventType,
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    const urls = tracking.events[event] || [];
    return this.beaconManager.fireMany(urls, context);
  }

  /**
   * Fire click tracking
   */
  async fireClickTracking(
    tracking: FlattenedTracking,
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    return this.beaconManager.fireMany(tracking.clickTrackings, context);
  }

  /**
   * Fire error tracking
   */
  async fireError(
    tracking: FlattenedTracking,
    errorCode: number,
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    const contextWithError = {
      ...context,
      macros: {
        ...context?.macros,
        ERRORCODE: errorCode.toString(),
      },
    };

    return this.beaconManager.fireMany(tracking.errors, contextWithError);
  }

  /**
   * Fire progress events based on playhead position
   */
  async fireProgressEvents(
    creative: SSAIReadyCreative,
    playheadSeconds: number,
    context?: TrackingContext
  ): Promise<TrackingBatchResult[]> {
    const results: TrackingBatchResult[] = [];
    const creativeId = creative.id;

    if (!this.trackingState.has(creativeId)) {
      // Memory protection: evict oldest entries if at limit
      if (this.trackingState.size >= MAX_TRACKING_STATES) {
        const evictCount = Math.floor(MAX_TRACKING_STATES / 4); // Evict 25%
        for (let i = 0; i < evictCount && this.trackingStateOrder.length > 0; i++) {
          const oldId = this.trackingStateOrder.shift();
          if (oldId) {
            this.trackingState.delete(oldId);
          }
        }
        logger.debug('[VASTTrackingManager] Evicted old tracking states (memory protection)', {
          evicted: evictCount,
          remaining: this.trackingState.size,
        });
      }
      this.trackingState.set(creativeId, new Set());
      this.trackingStateOrder.push(creativeId);
    }

    const firedEvents = this.trackingState.get(creativeId)!;
    
    // Use a small epsilon for floating-point comparisons (10ms tolerance)
    const PROGRESS_EPSILON = 0.01;
    const progress = playheadSeconds / creative.duration;

    // Define progress milestones
    const milestones: Array<{ progress: number; event: VASTTrackingEventType }> = [
      { progress: 0, event: 'start' },
      { progress: 0.25, event: 'firstQuartile' },
      { progress: 0.5, event: 'midpoint' },
      { progress: 0.75, event: 'thirdQuartile' },
      { progress: 1, event: 'complete' },
    ];

    for (const milestone of milestones) {
      // Use epsilon tolerance to handle floating-point edge cases
      // (e.g., playhead at 29.997s when duration is 30s should fire complete)
      if (progress >= milestone.progress - PROGRESS_EPSILON && !firedEvents.has(milestone.event)) {
        firedEvents.add(milestone.event);

        const urls = creative.tracking.events[milestone.event] || [];
        if (urls.length > 0) {
          const result = await this.beaconManager.fireMany(urls, context);
          result.event = milestone.event;
          results.push(result);
        }
      }
    }

    // Handle time-based progress tracking
    const progressUrls = creative.tracking.events.progress || [];
    for (const url of progressUrls) {
      // Check for offset in URL (simplified - full impl would parse from tracking data)
      const offsetMatch = url.match(/offset=(\d+)/);
      if (offsetMatch) {
        const offsetSeconds = parseInt(offsetMatch[1], 10);
        const eventKey = `progress_${offsetSeconds}` as VASTTrackingEventType;

        if (playheadSeconds >= offsetSeconds && !firedEvents.has(eventKey)) {
          firedEvents.add(eventKey);
          const result = await this.beaconManager.fire(url, context);
          results.push({
            event: eventKey,
            totalUrls: 1,
            successCount: result.success ? 1 : 0,
            failureCount: result.success ? 0 : 1,
            results: [result],
          });
        }
      }
    }

    return results;
  }

  /**
   * Fire viewability events
   */
  async fireViewability(
    viewableImpression: VASTViewableImpression,
    state: 'viewable' | 'notViewable' | 'viewUndetermined',
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    let urls: string[] = [];

    switch (state) {
      case 'viewable':
        urls = viewableImpression.viewable || [];
        break;
      case 'notViewable':
        urls = viewableImpression.notViewable || [];
        break;
      case 'viewUndetermined':
        urls = viewableImpression.viewUndetermined || [];
        break;
    }

    const result = await this.beaconManager.fireMany(urls, context);
    result.event = state;
    return result;
  }

  /**
   * Fire verification not executed event
   */
  async fireVerificationNotExecuted(
    verification: VASTVerification,
    reason: string,
    context?: TrackingContext
  ): Promise<TrackingBatchResult> {
    const urls = verification.trackingEvents
      .filter((te) => te.event === 'verificationNotExecuted')
      .map((te) => te.url);

    const contextWithReason = {
      ...context,
      macros: {
        ...context?.macros,
        REASON: reason,
      },
    };

    const result = await this.beaconManager.fireMany(urls, contextWithReason);
    result.event = 'verificationNotExecuted';
    return result;
  }

  /**
   * Reset tracking state for a creative
   */
  resetCreativeState(creativeId: string): void {
    this.trackingState.delete(creativeId);
    const idx = this.trackingStateOrder.indexOf(creativeId);
    if (idx !== -1) {
      this.trackingStateOrder.splice(idx, 1);
    }
  }

  /**
   * Reset all tracking state
   */
  resetAllState(): void {
    this.trackingState.clear();
    this.trackingStateOrder = [];
    this.beaconManager.clearFiredUrls();
  }

  /**
   * Cancel all pending beacons (e.g., on session error)
   */
  cancelAllPending(): void {
    this.beaconManager.cancelAll();
  }

  /**
   * Full cleanup on session end or error
   * Cancels pending beacons and resets all state
   */
  cleanup(): void {
    this.beaconManager.cancelAll();
    this.trackingState.clear();
    this.trackingStateOrder = [];
    this.beaconManager.clearFiredUrls();
    logger.debug('[VASTTrackingManager] Session cleanup complete');
  }

  /**
   * Get the underlying beacon manager
   */
  getBeaconManager(): BeaconManager {
    return this.beaconManager;
  }
}

// =============================================================================
// Utility Functions
// =============================================================================

/**
 * Format playhead position for VAST macro
 */
export function formatPlayhead(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);
  const millis = Math.round((seconds % 1) * 1000);

  return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}.${millis.toString().padStart(3, '0')}`;
}

/**
 * Calculate which quartile a playhead position is in
 */
export function getQuartile(
  playheadSeconds: number,
  durationSeconds: number
): 'start' | 'firstQuartile' | 'midpoint' | 'thirdQuartile' | 'complete' | null {
  if (durationSeconds <= 0) {
    return null;
  }

  const progress = playheadSeconds / durationSeconds;

  if (progress >= 1) return 'complete';
  if (progress >= 0.75) return 'thirdQuartile';
  if (progress >= 0.5) return 'midpoint';
  if (progress >= 0.25) return 'firstQuartile';
  if (progress >= 0) return 'start';

  return null;
}

/**
 * Build tracking context from session information
 */
export function buildTrackingContext(
  sessionId: string,
  options: {
    contentUrl?: string;
    userAgent?: string;
    clientIp?: string;
    domain?: string;
    pageUrl?: string;
    appBundle?: string;
    playhead?: number;
    adDuration?: number;
    podSequence?: number;
    adPosition?: number;
    assetUri?: string;
    playerWidth?: number;
    playerHeight?: number;
    playerState?: string;
    correlationId?: string;
  } = {}
): TrackingContext {
  const macros: Record<string, string> = {};

  if (options.domain) {
    macros.DOMAIN = options.domain;
  }
  if (options.pageUrl) {
    macros.PAGEURL = options.pageUrl;
  }
  if (options.appBundle) {
    macros.APPBUNDLE = options.appBundle;
  }
  if (options.playhead !== undefined) {
    macros.CONTENTPLAYHEAD = formatPlayhead(options.playhead);
    macros.ADPLAYHEAD = formatPlayhead(options.playhead);
  }
  if (options.adDuration !== undefined) {
    macros.DURATION = options.adDuration.toString();
  }
  if (options.podSequence !== undefined) {
    macros.PODSEQUENCE = options.podSequence.toString();
  }
  if (options.adPosition !== undefined) {
    macros.ADPOSITION = options.adPosition.toString();
  }
  if (options.assetUri) {
    macros.ASSETURI = options.assetUri;
  }
  if (options.playerWidth !== undefined && options.playerHeight !== undefined) {
    macros.PLAYERSIZE = `${options.playerWidth}x${options.playerHeight}`;
  }
  if (options.playerState) {
    macros.PLAYERSTATE = options.playerState;
  }

  return {
    sessionId,
    contentUrl: options.contentUrl,
    userAgent: options.userAgent,
    clientIp: options.clientIp,
    macros,
    correlationId: options.correlationId || sessionId,
  };
}

/**
 * Extract all tracking URLs from a creative
 */
export function getAllTrackingUrls(creative: SSAIReadyCreative): string[] {
  const urls: string[] = [];

  // Impressions
  urls.push(...creative.tracking.impressions);

  // Errors
  urls.push(...creative.tracking.errors);

  // Events
  for (const eventUrls of Object.values(creative.tracking.events)) {
    urls.push(...eventUrls);
  }

  // Click tracking
  urls.push(...creative.tracking.clickTrackings);

  // Click through (if present)
  if (creative.tracking.clickThrough) {
    urls.push(creative.tracking.clickThrough);
  }

  // Verifications
  for (const verification of creative.tracking.verifications) {
    if (verification.javaScriptResource?.url) {
      urls.push(verification.javaScriptResource.url);
    }
    if (verification.executableResource?.url) {
      urls.push(verification.executableResource.url);
    }
    for (const te of verification.trackingEvents) {
      urls.push(te.url);
    }
  }

  // Viewable impression
  if (creative.tracking.viewableImpression) {
    urls.push(...(creative.tracking.viewableImpression.viewable || []));
    urls.push(...(creative.tracking.viewableImpression.notViewable || []));
    urls.push(...(creative.tracking.viewableImpression.viewUndetermined || []));
  }

  return urls;
}

// =============================================================================
// Exports
// =============================================================================

export default VASTTrackingManager;
