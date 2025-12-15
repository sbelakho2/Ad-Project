/**
 * SSAI Event Tracker
 *
 * Handles tracking of ad events including impressions, quartiles, completion, and errors.
 * Fires tracking pixels and maintains event state.
 */

import {
  TrackingEvent,
  TrackingEventType,
  TrackingResult,
  TrackingUrl,
  CreativeTracking,
  QuartilePosition,
} from './types';

export interface EventTrackerConfig {
  /** Timeout for tracking calls (ms) */
  trackingTimeoutMs: number;

  /** Maximum retries for failed tracking */
  maxRetries: number;

  /** Retry delay (ms) */
  retryDelayMs: number;

  /** Fire and forget mode */
  fireAndForget: boolean;

  /** Enable parallel firing */
  parallelFire: boolean;

  /** Maximum parallel requests */
  maxParallelRequests: number;
}

const DEFAULT_CONFIG: EventTrackerConfig = {
  trackingTimeoutMs: 2000,
  maxRetries: 2,
  retryDelayMs: 500,
  fireAndForget: true,
  parallelFire: true,
  maxParallelRequests: 10,
};

interface TrackedState {
  events: Set<string>;
  lastUpdate: Date;
}

export class EventTracker {
  private config: EventTrackerConfig;
  private creativeTracking: Map<string, CreativeTracking> = new Map();
  private firedEvents: Map<string, TrackedState> = new Map();
  private pendingRequests: Map<string, Promise<boolean>> = new Map();

  constructor(config: Partial<EventTrackerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Register tracking configuration for a creative
   */
  registerCreative(tracking: CreativeTracking): void {
    this.creativeTracking.set(tracking.creativeId, tracking);
    this.firedEvents.set(tracking.creativeId, {
      events: new Set(),
      lastUpdate: new Date(),
    });
  }

  /**
   * Calculate quartile positions for a creative
   */
  calculateQuartiles(duration: number): QuartilePosition[] {
    return [
      { type: 'firstQuartile', position: duration * 0.25, progress: 25 },
      { type: 'midpoint', position: duration * 0.5, progress: 50 },
      { type: 'thirdQuartile', position: duration * 0.75, progress: 75 },
      { type: 'complete', position: duration, progress: 100 },
    ];
  }

  /**
   * Check if a quartile event should fire
   */
  shouldFireQuartile(
    creativeId: string,
    currentTime: number,
    duration: number
  ): QuartilePosition | null {
    const state = this.firedEvents.get(creativeId);
    if (!state) return null;

    const quartiles = this.calculateQuartiles(duration);
    const progress = (currentTime / duration) * 100;

    for (const quartile of quartiles) {
      const eventKey = `${creativeId}:${quartile.type}`;
      if (!state.events.has(eventKey) && progress >= quartile.progress) {
        return quartile;
      }
    }

    return null;
  }

  /**
   * Track an event
   */
  async trackEvent(event: TrackingEvent): Promise<TrackingResult> {
    const tracking = this.creativeTracking.get(event.creativeId);
    if (!tracking) {
      return {
        success: false,
        eventId: event.eventId,
        isDuplicate: false,
        urlsCalled: 0,
        errors: [`No tracking registered for creative ${event.creativeId}`],
      };
    }

    // Check for duplicate
    const state = this.firedEvents.get(event.creativeId);
    const eventKey = `${event.creativeId}:${event.type}`;
    if (state?.events.has(eventKey)) {
      return {
        success: true,
        eventId: event.eventId,
        isDuplicate: true,
        urlsCalled: 0,
        errors: [],
      };
    }

    // Get URLs for this event type
    const urls = tracking.trackingUrls[event.type] || [];
    if (urls.length === 0) {
      // Mark as fired even with no URLs
      state?.events.add(eventKey);
      return {
        success: true,
        eventId: event.eventId,
        isDuplicate: false,
        urlsCalled: 0,
        errors: [],
      };
    }

    // Fire tracking pixels
    const result = await this.fireTrackingPixels(event, urls);

    // Mark as fired
    if (result.success && state) {
      state.events.add(eventKey);
      state.lastUpdate = new Date();
    }

    return result;
  }

  /**
   * Track impression event
   */
  async trackImpression(
    sessionId: string,
    podId: string,
    slotId: string,
    creativeId: string
  ): Promise<TrackingResult> {
    const event: TrackingEvent = {
      eventId: `${sessionId}:${slotId}:impression:${Date.now()}`,
      type: 'impression',
      sessionId,
      podId,
      slotId,
      creativeId,
      timestamp: new Date(),
      mediaTime: 0,
      source: 'server',
    };

    return this.trackEvent(event);
  }

  /**
   * Track start event
   */
  async trackStart(
    sessionId: string,
    podId: string,
    slotId: string,
    creativeId: string
  ): Promise<TrackingResult> {
    const event: TrackingEvent = {
      eventId: `${sessionId}:${slotId}:start:${Date.now()}`,
      type: 'start',
      sessionId,
      podId,
      slotId,
      creativeId,
      timestamp: new Date(),
      mediaTime: 0,
      progress: 0,
      source: 'server',
    };

    return this.trackEvent(event);
  }

  /**
   * Track quartile event
   */
  async trackQuartile(
    sessionId: string,
    podId: string,
    slotId: string,
    creativeId: string,
    quartile: QuartilePosition
  ): Promise<TrackingResult> {
    const event: TrackingEvent = {
      eventId: `${sessionId}:${slotId}:${quartile.type}:${Date.now()}`,
      type: quartile.type,
      sessionId,
      podId,
      slotId,
      creativeId,
      timestamp: new Date(),
      mediaTime: quartile.position,
      progress: quartile.progress,
      source: 'server',
    };

    return this.trackEvent(event);
  }

  /**
   * Track complete event
   */
  async trackComplete(
    sessionId: string,
    podId: string,
    slotId: string,
    creativeId: string,
    duration: number
  ): Promise<TrackingResult> {
    const event: TrackingEvent = {
      eventId: `${sessionId}:${slotId}:complete:${Date.now()}`,
      type: 'complete',
      sessionId,
      podId,
      slotId,
      creativeId,
      timestamp: new Date(),
      mediaTime: duration,
      progress: 100,
      source: 'server',
    };

    return this.trackEvent(event);
  }

  /**
   * Track error event
   */
  async trackError(
    sessionId: string,
    podId: string,
    slotId: string,
    creativeId: string,
    errorCode: string,
    errorMessage: string,
    fatal: boolean = false
  ): Promise<TrackingResult> {
    const event: TrackingEvent = {
      eventId: `${sessionId}:${slotId}:error:${Date.now()}`,
      type: 'error',
      sessionId,
      podId,
      slotId,
      creativeId,
      timestamp: new Date(),
      mediaTime: 0,
      error: {
        code: errorCode,
        message: errorMessage,
        fatal,
      },
      source: 'server',
    };

    return this.trackEvent(event);
  }

  /**
   * Fire tracking pixels for an event
   */
  private async fireTrackingPixels(
    event: TrackingEvent,
    urls: TrackingUrl[]
  ): Promise<TrackingResult> {
    const errors: string[] = [];
    let successCount = 0;

    const fireUrl = async (trackingUrl: TrackingUrl): Promise<boolean> => {
      try {
        const expandedUrl = this.expandMacros(trackingUrl.url, event);
        await this.callTrackingUrl(expandedUrl);
        return true;
      } catch (err) {
        const message = err instanceof Error ? err.message : 'Unknown error';
        if (trackingUrl.required) {
          errors.push(`Failed to call ${trackingUrl.url}: ${message}`);
        }
        return false;
      }
    };

    if (this.config.parallelFire) {
      // Fire in parallel with concurrency limit
      const results = await this.parallelLimit(
        urls.map((url) => () => fireUrl(url)),
        this.config.maxParallelRequests
      );
      successCount = results.filter((r) => r).length;
    } else {
      // Fire sequentially
      for (const url of urls) {
        if (await fireUrl(url)) {
          successCount++;
        }
      }
    }

    const requiredUrls = urls.filter((u) => u.required);
    const requiredSuccess = requiredUrls.length === 0 || errors.length === 0;

    return {
      success: requiredSuccess,
      eventId: event.eventId,
      isDuplicate: false,
      urlsCalled: successCount,
      errors,
    };
  }

  /**
   * Expand macros in tracking URL
   */
  private expandMacros(url: string, event: TrackingEvent): string {
    const macros: Record<string, string> = {
      '[SESSION_ID]': event.sessionId,
      '[POD_ID]': event.podId,
      '[SLOT_ID]': event.slotId,
      '[CREATIVE_ID]': event.creativeId,
      '[TIMESTAMP]': event.timestamp.toISOString(),
      '[MEDIA_TIME]': event.mediaTime.toString(),
      '[EVENT_TYPE]': event.type,
      '[CACHEBUSTER]': Math.random().toString(36).substring(2),
    };

    if (event.progress !== undefined) {
      macros['[PROGRESS]'] = event.progress.toString();
    }

    if (event.error) {
      macros['[ERROR_CODE]'] = event.error.code;
      macros['[ERROR_MESSAGE]'] = encodeURIComponent(event.error.message);
    }

    let expandedUrl = url;
    for (const [macro, value] of Object.entries(macros)) {
      expandedUrl = expandedUrl.replace(new RegExp(macro.replace(/\[/g, '\\[').replace(/\]/g, '\\]'), 'g'), value);
    }

    return expandedUrl;
  }

  /**
   * Call a tracking URL with timeout and retry
   */
  private async callTrackingUrl(url: string): Promise<void> {
    // Check for pending request
    if (this.pendingRequests.has(url)) {
      await this.pendingRequests.get(url);
      return;
    }

    const attempt = async (retryCount: number): Promise<boolean> => {
      try {
        // In real implementation, use fetch or HTTP client
        // For now, simulate the call
        await this.simulateHttpCall(url);
        return true;
      } catch (err) {
        if (retryCount < this.config.maxRetries) {
          await this.delay(this.config.retryDelayMs);
          return attempt(retryCount + 1);
        }
        throw err;
      }
    };

    const promise = attempt(0);
    this.pendingRequests.set(url, promise);

    try {
      await promise;
    } finally {
      this.pendingRequests.delete(url);
    }
  }

  /**
   * Simulate HTTP call (to be replaced with actual fetch)
   */
  private async simulateHttpCall(url: string): Promise<void> {
    // Placeholder for actual HTTP implementation
    // In production: fetch(url, { method: 'GET', signal: AbortSignal.timeout(this.config.trackingTimeoutMs) })
    await this.delay(10);
    
    // Simulate occasional failures for testing
    if (url.includes('fail-test')) {
      throw new Error('Simulated tracking failure');
    }
  }

  /**
   * Execute promises with concurrency limit
   */
  private async parallelLimit<T>(
    tasks: Array<() => Promise<T>>,
    limit: number
  ): Promise<T[]> {
    const results: T[] = [];
    const executing: Promise<void>[] = [];

    for (const task of tasks) {
      const p = task().then((result) => {
        results.push(result);
      });

      const e = p.then(() => {
        executing.splice(executing.indexOf(e), 1);
      });
      executing.push(e);

      if (executing.length >= limit) {
        await Promise.race(executing);
      }
    }

    await Promise.all(executing);
    return results;
  }

  /**
   * Delay helper
   */
  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  /**
   * Get fired events for a creative
   */
  getFiredEvents(creativeId: string): TrackingEventType[] {
    const state = this.firedEvents.get(creativeId);
    if (!state) return [];

    const events: TrackingEventType[] = [];
    for (const key of state.events) {
      const eventType = key.split(':')[1] as TrackingEventType;
      if (eventType) events.push(eventType);
    }
    return events;
  }

  /**
   * Clear tracking state (for testing)
   */
  clearState(): void {
    this.creativeTracking.clear();
    this.firedEvents.clear();
    this.pendingRequests.clear();
  }
}
