/**
 * Ad Tracking Service
 * Records and retrieves ad tracking events and metrics
 */

export interface ImpressionEvent {
  sessionId: string;
  podId: string;
  creativeId: string;
  timestamp: number;
  advertiserId?: string;
  campaignId?: string;
}

export interface TrackingResult {
  recorded: boolean;
  eventId?: string;
  error?: string;
}

export interface SessionMetrics {
  sessionId: string;
  impressions: number;
  quartiles: {
    start: number;
    firstQuartile: number;
    midpoint: number;
    thirdQuartile: number;
    complete: number;
  };
  clicks: number;
  errors: number;
  startTime: number;
  lastActivityTime: number;
}

export class AdTrackingService {
  private impressions = new Map<string, ImpressionEvent[]>();
  private sessionMetrics = new Map<string, SessionMetrics>();
  private eventIdCounter = 0;

  /**
   * Record an impression event
   */
  async recordImpression(event: ImpressionEvent): Promise<TrackingResult> {
    try {
      const eventId = this.generateEventId();
      
      // Store impression
      if (!this.impressions.has(event.sessionId)) {
        this.impressions.set(event.sessionId, []);
      }
      this.impressions.get(event.sessionId)!.push(event);

      // Update session metrics
      this.updateSessionMetrics(event.sessionId, 'impression');

      return { recorded: true, eventId };
    } catch (error) {
      return { 
        recorded: false, 
        error: error instanceof Error ? error.message : 'Unknown error' 
      };
    }
  }

  /**
   * Record quartile event
   */
  async recordQuartile(
    sessionId: string,
    quartile: 'start' | 'firstQuartile' | 'midpoint' | 'thirdQuartile' | 'complete'
  ): Promise<TrackingResult> {
    try {
      this.updateSessionMetrics(sessionId, quartile);
      return { recorded: true, eventId: this.generateEventId() };
    } catch (error) {
      return { 
        recorded: false, 
        error: error instanceof Error ? error.message : 'Unknown error' 
      };
    }
  }

  /**
   * Record click event
   */
  async recordClick(sessionId: string, creativeId: string): Promise<TrackingResult> {
    try {
      this.updateSessionMetrics(sessionId, 'click');
      return { recorded: true, eventId: this.generateEventId() };
    } catch (error) {
      return { 
        recorded: false, 
        error: error instanceof Error ? error.message : 'Unknown error' 
      };
    }
  }

  /**
   * Get session metrics
   */
  async getSessionMetrics(sessionId: string): Promise<SessionMetrics> {
    const metrics = this.sessionMetrics.get(sessionId);
    
    if (!metrics) {
      return {
        sessionId,
        impressions: 0,
        quartiles: {
          start: 0,
          firstQuartile: 0,
          midpoint: 0,
          thirdQuartile: 0,
          complete: 0,
        },
        clicks: 0,
        errors: 0,
        startTime: Date.now(),
        lastActivityTime: Date.now(),
      };
    }

    return metrics;
  }

  /**
   * Get impressions for session
   */
  async getSessionImpressions(sessionId: string): Promise<ImpressionEvent[]> {
    return this.impressions.get(sessionId) || [];
  }

  /**
   * Clean up session data
   */
  async cleanupSession(sessionId: string): Promise<void> {
    this.impressions.delete(sessionId);
    this.sessionMetrics.delete(sessionId);
  }

  private updateSessionMetrics(sessionId: string, eventType: string): void {
    let metrics = this.sessionMetrics.get(sessionId);
    
    if (!metrics) {
      metrics = {
        sessionId,
        impressions: 0,
        quartiles: {
          start: 0,
          firstQuartile: 0,
          midpoint: 0,
          thirdQuartile: 0,
          complete: 0,
        },
        clicks: 0,
        errors: 0,
        startTime: Date.now(),
        lastActivityTime: Date.now(),
      };
      this.sessionMetrics.set(sessionId, metrics);
    }

    metrics.lastActivityTime = Date.now();

    switch (eventType) {
      case 'impression':
        metrics.impressions++;
        break;
      case 'start':
        metrics.quartiles.start++;
        break;
      case 'firstQuartile':
        metrics.quartiles.firstQuartile++;
        break;
      case 'midpoint':
        metrics.quartiles.midpoint++;
        break;
      case 'thirdQuartile':
        metrics.quartiles.thirdQuartile++;
        break;
      case 'complete':
        metrics.quartiles.complete++;
        break;
      case 'click':
        metrics.clicks++;
        break;
      case 'error':
        metrics.errors++;
        break;
    }
  }

  private generateEventId(): string {
    return `evt-${Date.now()}-${++this.eventIdCounter}`;
  }
}

export default AdTrackingService;
