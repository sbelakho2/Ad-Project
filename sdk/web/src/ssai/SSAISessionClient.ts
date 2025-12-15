/**
 * SSAI Session Client for Web SDK
 *
 * Manages SSAI session lifecycle including creation, tracking, and cleanup.
 */

import {
  SSAIConfig,
  SSAISession,
  SSAITrackingEvent,
  SSAITrackingEventType,
  SSAISessionState,
  SSAIMetrics,
  SSAIError,
} from './types';

export interface SSAISessionClientConfig {
  /** Request timeout (ms) */
  requestTimeoutMs: number;

  /** Retry count for failed requests */
  maxRetries: number;

  /** Retry backoff base (ms) */
  retryBackoffMs: number;

  /** Enable tracking batching */
  enableBatching: boolean;

  /** Batch interval (ms) */
  batchIntervalMs: number;

  /** Max events per batch */
  maxBatchSize: number;
}

const DEFAULT_CONFIG: SSAISessionClientConfig = {
  requestTimeoutMs: 5000,
  maxRetries: 2,
  retryBackoffMs: 100,
  enableBatching: true,
  batchIntervalMs: 1000,
  maxBatchSize: 10,
};

export class SSAISessionClient {
  private config: SSAISessionClientConfig;
  private ssaiConfig: SSAIConfig | null = null;
  private session: SSAISession | null = null;
  private state: SSAISessionState = 'idle';
  private eventQueue: SSAITrackingEvent[] = [];
  private batchTimer: ReturnType<typeof setInterval> | null = null;
  private firedEvents: Set<string> = new Set();
  private metrics: SSAIMetrics = {
    adBreakCount: 0,
    adCount: 0,
    totalAdTime: 0,
    completedAds: 0,
    skippedAds: 0,
    erroredAds: 0,
    fillRate: 0,
  };

  constructor(config: Partial<SSAISessionClientConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Get current session state
   */
  getState(): SSAISessionState {
    return this.state;
  }

  /**
   * Get current session
   */
  getSession(): SSAISession | null {
    return this.session;
  }

  /**
   * Get metrics
   */
  getMetrics(): SSAIMetrics {
    return { ...this.metrics };
  }

  /**
   * Create a new SSAI session
   */
  async createSession(config: SSAIConfig): Promise<SSAISession> {
    this.state = 'initializing';
    this.ssaiConfig = config;

    try {
      const response = await this.fetchWithRetry(
        `${config.endpoint}/v1/ssai/session`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            tenantId: config.tenantId,
            contentUrl: config.contentUrl,
            format: config.format,
            capabilities: config.capabilities,
            consent: config.consent,
          }),
        }
      );

      if (!response.ok) {
        const errorData = await response.json().catch(() => ({}));
        throw {
          code: `HTTP_${response.status}`,
          message: errorData.message || 'Failed to create session',
          fatal: true,
        } as SSAIError;
      }

      const data = await response.json();

      this.session = {
        sessionId: data.sessionId,
        manifestUrl: data.manifestUrl,
        sessionToken: data.sessionToken,
        trackingUrl: data.trackingUrl,
        expiresAt: new Date(data.expiresAt),
        contentUrl: config.contentUrl,
      };

      this.state = 'active';
      this.startBatching();

      return this.session;
    } catch (error) {
      this.state = 'error';
      throw error;
    }
  }

  /**
   * Track a SSAI event
   */
  trackEvent(event: Omit<SSAITrackingEvent, 'timestamp'>): void {
    if (!this.session) {
      console.warn('[SSAI] Cannot track event without active session');
      return;
    }

    // Deduplicate
    const eventKey = `${event.type}:${event.slotId}`;
    if (this.shouldDedupe(event.type) && this.firedEvents.has(eventKey)) {
      return;
    }

    const fullEvent: SSAITrackingEvent = {
      ...event,
      timestamp: new Date(),
    };

    if (this.config.enableBatching) {
      this.eventQueue.push(fullEvent);
      if (this.eventQueue.length >= this.config.maxBatchSize) {
        this.flushEvents();
      }
    } else {
      this.sendEvent(fullEvent);
    }

    // Mark as fired
    if (this.shouldDedupe(event.type)) {
      this.firedEvents.add(eventKey);
    }

    // Update metrics
    this.updateMetrics(event.type);
  }

  /**
   * Track impression
   */
  trackImpression(podId: string, slotId: string, creativeId: string, mediaTime: number): void {
    this.trackEvent({
      type: 'impression',
      podId,
      slotId,
      creativeId,
      mediaTime,
    });
  }

  /**
   * Track quartile progress
   */
  trackProgress(
    podId: string,
    slotId: string,
    creativeId: string,
    mediaTime: number,
    progress: number
  ): void {
    // Determine quartile
    let eventType: SSAITrackingEventType | null = null;

    if (progress >= 100) {
      eventType = 'complete';
    } else if (progress >= 75) {
      eventType = 'thirdQuartile';
    } else if (progress >= 50) {
      eventType = 'midpoint';
    } else if (progress >= 25) {
      eventType = 'firstQuartile';
    } else if (progress >= 0) {
      eventType = 'start';
    }

    if (eventType) {
      this.trackEvent({
        type: eventType,
        podId,
        slotId,
        creativeId,
        mediaTime,
        metadata: { progress },
      });
    }
  }

  /**
   * Track error
   */
  trackError(
    podId: string,
    slotId: string,
    creativeId: string,
    errorCode: string,
    errorMessage: string
  ): void {
    this.trackEvent({
      type: 'error',
      podId,
      slotId,
      creativeId,
      mediaTime: 0,
      metadata: { errorCode, errorMessage },
    });
  }

  /**
   * Track skip
   */
  trackSkip(podId: string, slotId: string, creativeId: string, mediaTime: number): void {
    this.trackEvent({
      type: 'skip',
      podId,
      slotId,
      creativeId,
      mediaTime,
    });
  }

  /**
   * Track click
   */
  trackClick(podId: string, slotId: string, creativeId: string, mediaTime: number): void {
    this.trackEvent({
      type: 'click',
      podId,
      slotId,
      creativeId,
      mediaTime,
    });
  }

  /**
   * Flush pending events
   */
  async flushEvents(): Promise<void> {
    if (this.eventQueue.length === 0) return;

    const events = this.eventQueue.splice(0, this.config.maxBatchSize);
    await this.sendEventBatch(events);
  }

  /**
   * End the session
   */
  async endSession(): Promise<void> {
    // Flush remaining events
    await this.flushEvents();
    this.stopBatching();

    // Notify server of session end
    if (this.session) {
      try {
        await this.fetchWithRetry(
          `${this.ssaiConfig?.endpoint}/v1/ssai/session/${this.session.sessionId}/end`,
          { method: 'POST' }
        );
      } catch (error) {
        console.warn('[SSAI] Failed to end session gracefully', error);
      }
    }

    this.session = null;
    this.state = 'idle';
    this.firedEvents.clear();
    this.resetMetrics();
  }

  /**
   * Check if session is valid
   */
  isSessionValid(): boolean {
    if (!this.session) return false;
    return new Date() < this.session.expiresAt;
  }

  /**
   * Refresh session if needed
   */
  async refreshIfNeeded(): Promise<void> {
    if (!this.session || !this.ssaiConfig) return;

    // Refresh if less than 5 minutes remaining
    const remaining = this.session.expiresAt.getTime() - Date.now();
    if (remaining < 5 * 60 * 1000) {
      try {
        const response = await this.fetchWithRetry(
          `${this.ssaiConfig.endpoint}/v1/ssai/session/${this.session.sessionId}/refresh`,
          { method: 'POST' }
        );

        if (response.ok) {
          const data = await response.json();
          this.session.expiresAt = new Date(data.expiresAt);
          this.session.sessionToken = data.sessionToken;
        }
      } catch (error) {
        console.warn('[SSAI] Failed to refresh session', error);
      }
    }
  }

  /**
   * Start event batching
   */
  private startBatching(): void {
    if (!this.config.enableBatching) return;

    this.batchTimer = setInterval(() => {
      this.flushEvents();
    }, this.config.batchIntervalMs);
  }

  /**
   * Stop event batching
   */
  private stopBatching(): void {
    if (this.batchTimer) {
      clearInterval(this.batchTimer);
      this.batchTimer = null;
    }
  }

  /**
   * Send a single event
   */
  private async sendEvent(event: SSAITrackingEvent): Promise<void> {
    if (!this.session) return;

    try {
      await fetch(
        `${this.session.trackingUrl}/event`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'X-Session-Token': this.session.sessionToken,
          },
          body: JSON.stringify(event),
          keepalive: true, // Allow sending after page unload
        }
      );
    } catch (error) {
      console.warn('[SSAI] Failed to send tracking event', error);
    }
  }

  /**
   * Send a batch of events
   */
  private async sendEventBatch(events: SSAITrackingEvent[]): Promise<void> {
    if (!this.session || events.length === 0) return;

    try {
      await fetch(
        `${this.session.trackingUrl}/events`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'X-Session-Token': this.session.sessionToken,
          },
          body: JSON.stringify({ events }),
          keepalive: true,
        }
      );
    } catch (error) {
      console.warn('[SSAI] Failed to send tracking batch', error);
    }
  }

  /**
   * Fetch with retry logic
   */
  private async fetchWithRetry(
    url: string,
    options: RequestInit
  ): Promise<Response> {
    let lastError: Error | null = null;

    for (let attempt = 0; attempt <= this.config.maxRetries; attempt++) {
      try {
        const controller = new AbortController();
        const timeoutId = setTimeout(
          () => controller.abort(),
          this.config.requestTimeoutMs
        );

        const response = await fetch(url, {
          ...options,
          signal: controller.signal,
        });

        clearTimeout(timeoutId);
        return response;
      } catch (error) {
        lastError = error instanceof Error ? error : new Error(String(error));

        if (attempt < this.config.maxRetries) {
          await this.delay(this.config.retryBackoffMs * Math.pow(2, attempt));
        }
      }
    }

    throw lastError;
  }

  /**
   * Check if event type should be deduplicated
   */
  private shouldDedupe(eventType: SSAITrackingEventType): boolean {
    return [
      'impression',
      'start',
      'firstQuartile',
      'midpoint',
      'thirdQuartile',
      'complete',
    ].includes(eventType);
  }

  /**
   * Update metrics based on event
   */
  private updateMetrics(eventType: SSAITrackingEventType): void {
    switch (eventType) {
      case 'impression':
        this.metrics.adCount++;
        break;
      case 'complete':
        this.metrics.completedAds++;
        break;
      case 'skip':
        this.metrics.skippedAds++;
        break;
      case 'error':
        this.metrics.erroredAds++;
        break;
    }
  }

  /**
   * Reset metrics
   */
  private resetMetrics(): void {
    this.metrics = {
      adBreakCount: 0,
      adCount: 0,
      totalAdTime: 0,
      completedAds: 0,
      skippedAds: 0,
      erroredAds: 0,
      fillRate: 0,
    };
  }

  /**
   * Delay helper
   */
  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
}
