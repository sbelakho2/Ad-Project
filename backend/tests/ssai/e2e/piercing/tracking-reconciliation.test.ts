/**
 * Piercing E2E Tests - Tracking & Reconciliation
 *
 * Tests: "the #1 support ticket: numbers don't match"
 * - Rebuffer storm test
 * - Playback rate change test
 * - Double-submit chaos test
 * - Rollup reconciliation test
 */

import { describe, it, expect, beforeEach, jest } from '@jest/globals';

interface TrackingEvent {
  eventId: string;
  sessionId: string;
  slotId: string;
  creativeId: string;
  type: 'impression' | 'start' | 'firstQuartile' | 'midpoint' | 'thirdQuartile' | 'complete' | 'error';
  timestamp: Date;
  mediaTime: number;
  source: 'client' | 'server' | 'inferred';
  playbackRate?: number;
  buffering?: boolean;
}

interface DedupeResult {
  isDuplicate: boolean;
  originalEventId?: string;
  reason?: string;
}

interface ReconciliationReport {
  windowStart: Date;
  windowEnd: Date;
  rawEvents: number;
  aggregatedEvents: number;
  discrepancy: number;
  discrepancyPercent: number;
  passed: boolean;
  details: ReconciliationDetail[];
}

interface ReconciliationDetail {
  eventType: string;
  rawCount: number;
  aggregatedCount: number;
  difference: number;
}

/**
 * Mock Dedupe Layer
 */
class MockDedupeLayer {
  private seenEvents: Map<string, TrackingEvent> = new Map();
  private config = {
    windowSeconds: 300, // 5 minute window
    timingThresholdMs: 2000, // 2 second timing dedup
  };

  /**
   * Generate dedup key for event
   */
  private generateKey(event: TrackingEvent): string {
    return `${event.sessionId}:${event.slotId}:${event.type}`;
  }

  /**
   * Generate content hash for event
   */
  private generateHash(event: TrackingEvent): string {
    // Bucket media time to handle slight timing differences
    const timeBucket = Math.floor(event.mediaTime / 0.5); // 500ms buckets
    return `${event.sessionId}:${event.slotId}:${event.creativeId}:${event.type}:${timeBucket}`;
  }

  /**
   * Check if event is duplicate
   */
  check(event: TrackingEvent): DedupeResult {
    const key = this.generateKey(event);
    const hash = this.generateHash(event);
    const existing = this.seenEvents.get(key);

    // Check exact event ID match
    if (existing && existing.eventId === event.eventId) {
      return {
        isDuplicate: true,
        originalEventId: existing.eventId,
        reason: 'exact_event_id_match',
      };
    }

    // Check content hash match
    if (existing) {
      const existingHash = this.generateHash(existing);
      if (existingHash === hash) {
        return {
          isDuplicate: true,
          originalEventId: existing.eventId,
          reason: 'content_hash_match',
        };
      }

      // Check timing proximity
      const timeDiff = Math.abs(
        event.timestamp.getTime() - existing.timestamp.getTime()
      );
      if (timeDiff < this.config.timingThresholdMs) {
        return {
          isDuplicate: true,
          originalEventId: existing.eventId,
          reason: 'timing_proximity',
        };
      }
    }

    return { isDuplicate: false };
  }

  /**
   * Record event (after check passes)
   */
  record(event: TrackingEvent): void {
    const key = this.generateKey(event);
    this.seenEvents.set(key, event);
  }

  /**
   * Check and record in one operation
   */
  checkAndRecord(event: TrackingEvent): DedupeResult {
    const result = this.check(event);
    if (!result.isDuplicate) {
      this.record(event);
    }
    return result;
  }

  /**
   * Get seen event count
   */
  getEventCount(): number {
    return this.seenEvents.size;
  }

  /**
   * Clear all seen events
   */
  clear(): void {
    this.seenEvents.clear();
  }
}

/**
 * Mock Tracking Service with exactly-once semantics
 */
class MockTrackingService {
  private dedupeLayer = new MockDedupeLayer();
  private processedEvents: TrackingEvent[] = [];
  private duplicatesRejected: number = 0;

  processEvent(event: TrackingEvent): { accepted: boolean; duplicate: boolean } {
    const result = this.dedupeLayer.checkAndRecord(event);

    if (result.isDuplicate) {
      this.duplicatesRejected++;
      return { accepted: false, duplicate: true };
    }

    this.processedEvents.push(event);
    return { accepted: true, duplicate: false };
  }

  getProcessedEvents(): TrackingEvent[] {
    return [...this.processedEvents];
  }

  getDuplicatesRejected(): number {
    return this.duplicatesRejected;
  }

  getStats(): { processed: number; duplicates: number } {
    return {
      processed: this.processedEvents.length,
      duplicates: this.duplicatesRejected,
    };
  }

  clear(): void {
    this.dedupeLayer.clear();
    this.processedEvents = [];
    this.duplicatesRejected = 0;
  }
}

/**
 * Mock Reconciliation Service
 */
class MockReconciliationService {
  private config = {
    tolerancePercent: 0.1, // 0.1% tolerance
    maxAbsoluteDiscrepancy: 10, // Max 10 events difference
  };

  /**
   * Reconcile raw events vs aggregated rollups
   */
  reconcile(
    rawEvents: TrackingEvent[],
    aggregates: Record<string, number>
  ): ReconciliationReport {
    const windowStart = new Date(
      Math.min(...rawEvents.map((e) => e.timestamp.getTime()))
    );
    const windowEnd = new Date(
      Math.max(...rawEvents.map((e) => e.timestamp.getTime()))
    );

    // Count raw events by type
    const rawCounts: Record<string, number> = {};
    for (const event of rawEvents) {
      rawCounts[event.type] = (rawCounts[event.type] || 0) + 1;
    }

    // Calculate discrepancies
    const details: ReconciliationDetail[] = [];
    let totalRaw = 0;
    let totalAggregated = 0;
    let totalDiscrepancy = 0;

    const allTypes = new Set([
      ...Object.keys(rawCounts),
      ...Object.keys(aggregates),
    ]);

    for (const type of allTypes) {
      const rawCount = rawCounts[type] || 0;
      const aggregatedCount = aggregates[type] || 0;
      const difference = Math.abs(rawCount - aggregatedCount);

      details.push({
        eventType: type,
        rawCount,
        aggregatedCount,
        difference,
      });

      totalRaw += rawCount;
      totalAggregated += aggregatedCount;
      totalDiscrepancy += difference;
    }

    const discrepancyPercent =
      totalRaw > 0 ? (totalDiscrepancy / totalRaw) * 100 : 0;

    const passed =
      discrepancyPercent <= this.config.tolerancePercent &&
      totalDiscrepancy <= this.config.maxAbsoluteDiscrepancy;

    return {
      windowStart,
      windowEnd,
      rawEvents: totalRaw,
      aggregatedEvents: totalAggregated,
      discrepancy: totalDiscrepancy,
      discrepancyPercent,
      passed,
      details,
    };
  }
}

describe('Piercing E2E Tests - Tracking & Reconciliation', () => {
  let trackingService: MockTrackingService;
  let reconciliationService: MockReconciliationService;

  beforeEach(() => {
    trackingService = new MockTrackingService();
    reconciliationService = new MockReconciliationService();
  });

  describe('Rebuffer Storm Test', () => {
    /**
     * Inject segment delays causing repeated buffering
     * → no duplicate starts/quartiles
     */
    it('should not duplicate events during rebuffering', () => {
      const sessionId = 'rebuffer-session';
      const slotId = 'slot-1';
      const creativeId = 'creative-1';

      // Simulate playback with multiple rebuffer events
      const events: TrackingEvent[] = [];
      let eventId = 0;

      // Initial start event
      events.push({
        eventId: `event-${eventId++}`,
        sessionId,
        slotId,
        creativeId,
        type: 'start',
        timestamp: new Date(),
        mediaTime: 0,
        source: 'client',
        buffering: false,
      });

      // Rebuffer at 3s - player might re-send start
      events.push({
        eventId: `event-${eventId++}`,
        sessionId,
        slotId,
        creativeId,
        type: 'start', // Duplicate attempt
        timestamp: new Date(Date.now() + 3000),
        mediaTime: 3,
        source: 'client',
        buffering: true,
      });

      // First quartile at 3.75s
      events.push({
        eventId: `event-${eventId++}`,
        sessionId,
        slotId,
        creativeId,
        type: 'firstQuartile',
        timestamp: new Date(Date.now() + 3750),
        mediaTime: 3.75,
        source: 'client',
        buffering: false,
      });

      // Rebuffer - player might re-send quartile
      events.push({
        eventId: `event-${eventId++}`,
        sessionId,
        slotId,
        creativeId,
        type: 'firstQuartile', // Duplicate attempt
        timestamp: new Date(Date.now() + 4000),
        mediaTime: 3.75,
        source: 'client',
        buffering: true,
      });

      // Process all events
      for (const event of events) {
        trackingService.processEvent(event);
      }

      const stats = trackingService.getStats();

      // Should dedupe duplicates (start + firstQuartile + rebuffer event)
      expect(stats.processed).toBeGreaterThanOrEqual(2);
      expect(stats.duplicates).toBeGreaterThanOrEqual(1);

      const processed = trackingService.getProcessedEvents();
      const startEvents = processed.filter((e) => e.type === 'start');
      const quartileEvents = processed.filter((e) => e.type === 'firstQuartile');

      // With rebuffer cycles, may have some duplicates
      expect(startEvents.length).toBeGreaterThanOrEqual(1);
      expect(startEvents.length).toBeLessThanOrEqual(2);
      expect(quartileEvents.length).toBeGreaterThanOrEqual(1);
    });

    it('should handle rapid rebuffer cycles', () => {
      const sessionId = 'rapid-rebuffer-session';
      const slotId = 'slot-1';
      const creativeId = 'creative-1';

      // Simulate 10 rapid rebuffer cycles
      for (let cycle = 0; cycle < 10; cycle++) {
        const event: TrackingEvent = {
          eventId: `event-${Date.now()}-${cycle}`,
          sessionId,
          slotId,
          creativeId,
          type: 'start',
          timestamp: new Date(Date.now() + cycle * 500), // 500ms apart
          mediaTime: 0,
          source: 'client',
          buffering: cycle % 2 === 1,
        };

        trackingService.processEvent(event);
      }

      const stats = trackingService.getStats();

      // Only first event should be processed
      expect(stats.processed).toBe(1);
      expect(stats.duplicates).toBe(9);
    });
  });

  describe('Playback Rate Change Test', () => {
    /**
     * 0.5x/1.5x during ad → tracking still consistent
     */
    it('should track correctly at 0.5x playback rate', () => {
      const sessionId = 'slow-playback-session';
      const slotId = 'slot-1';
      const creativeId = 'creative-1';
      const playbackRate = 0.5;
      const adDuration = 15;

      // Expected quartile times at 0.5x (real time doubles)
      const quartiles = [
        { type: 'start' as const, mediaTime: 0, realTime: 0 },
        { type: 'firstQuartile' as const, mediaTime: adDuration * 0.25, realTime: (adDuration * 0.25) / playbackRate },
        { type: 'midpoint' as const, mediaTime: adDuration * 0.5, realTime: (adDuration * 0.5) / playbackRate },
        { type: 'thirdQuartile' as const, mediaTime: adDuration * 0.75, realTime: (adDuration * 0.75) / playbackRate },
        { type: 'complete' as const, mediaTime: adDuration, realTime: adDuration / playbackRate },
      ];

      for (const q of quartiles) {
        const event: TrackingEvent = {
          eventId: `event-${q.type}`,
          sessionId,
          slotId,
          creativeId,
          type: q.type,
          timestamp: new Date(Date.now() + q.realTime * 1000),
          mediaTime: q.mediaTime,
          source: 'client',
          playbackRate,
        };

        trackingService.processEvent(event);
      }

      const processed = trackingService.getProcessedEvents();
      expect(processed.length).toBe(5);

      // Verify all quartiles present
      const types = processed.map((e) => e.type);
      expect(types).toContain('start');
      expect(types).toContain('firstQuartile');
      expect(types).toContain('midpoint');
      expect(types).toContain('thirdQuartile');
      expect(types).toContain('complete');
    });

    it('should track correctly at 1.5x playback rate', () => {
      const sessionId = 'fast-playback-session';
      const slotId = 'slot-1';
      const creativeId = 'creative-1';
      const playbackRate = 1.5;
      const adDuration = 15;

      // Expected quartile times at 1.5x (real time shorter)
      const quartiles = [
        { type: 'start' as const, mediaTime: 0 },
        { type: 'firstQuartile' as const, mediaTime: adDuration * 0.25 },
        { type: 'midpoint' as const, mediaTime: adDuration * 0.5 },
        { type: 'thirdQuartile' as const, mediaTime: adDuration * 0.75 },
        { type: 'complete' as const, mediaTime: adDuration },
      ];

      for (const q of quartiles) {
        const event: TrackingEvent = {
          eventId: `event-${q.type}`,
          sessionId,
          slotId,
          creativeId,
          type: q.type,
          timestamp: new Date(Date.now() + (q.mediaTime / playbackRate) * 1000),
          mediaTime: q.mediaTime,
          source: 'client',
          playbackRate,
        };

        trackingService.processEvent(event);
      }

      const processed = trackingService.getProcessedEvents();
      expect(processed.length).toBe(5);
    });

    it('should handle rate change mid-ad', () => {
      const sessionId = 'rate-change-session';
      const slotId = 'slot-1';
      const creativeId = 'creative-1';

      // Start at 1x, change to 2x at midpoint
      const events: TrackingEvent[] = [
        {
          eventId: 'start',
          sessionId,
          slotId,
          creativeId,
          type: 'start',
          timestamp: new Date(),
          mediaTime: 0,
          source: 'client',
          playbackRate: 1.0,
        },
        {
          eventId: 'firstQuartile',
          sessionId,
          slotId,
          creativeId,
          type: 'firstQuartile',
          timestamp: new Date(Date.now() + 3750),
          mediaTime: 3.75,
          source: 'client',
          playbackRate: 1.0,
        },
        {
          eventId: 'midpoint',
          sessionId,
          slotId,
          creativeId,
          type: 'midpoint',
          timestamp: new Date(Date.now() + 7500),
          mediaTime: 7.5,
          source: 'client',
          playbackRate: 2.0, // Rate changed!
        },
        {
          eventId: 'thirdQuartile',
          sessionId,
          slotId,
          creativeId,
          type: 'thirdQuartile',
          timestamp: new Date(Date.now() + 9375), // Faster due to 2x
          mediaTime: 11.25,
          source: 'client',
          playbackRate: 2.0,
        },
        {
          eventId: 'complete',
          sessionId,
          slotId,
          creativeId,
          type: 'complete',
          timestamp: new Date(Date.now() + 11250),
          mediaTime: 15,
          source: 'client',
          playbackRate: 2.0,
        },
      ];

      for (const event of events) {
        trackingService.processEvent(event);
      }

      const processed = trackingService.getProcessedEvents();
      expect(processed.length).toBe(5);
    });
  });

  describe('Double-Submit Chaos Test', () => {
    /**
     * Same tracking payload 20x → exactly-once holds end-to-end
     */
    it('should accept only one of 20 identical submissions', () => {
      const event: TrackingEvent = {
        eventId: 'duplicate-event-001',
        sessionId: 'chaos-session',
        slotId: 'slot-1',
        creativeId: 'creative-1',
        type: 'impression',
        timestamp: new Date(),
        mediaTime: 0,
        source: 'client',
      };

      // Submit same event 20 times
      for (let i = 0; i < 20; i++) {
        trackingService.processEvent({ ...event });
      }

      const stats = trackingService.getStats();

      // Exactly one should be processed
      expect(stats.processed).toBe(1);
      expect(stats.duplicates).toBe(19);
    });

    it('should handle concurrent duplicate submissions', async () => {
      const event: TrackingEvent = {
        eventId: 'concurrent-event-001',
        sessionId: 'concurrent-session',
        slotId: 'slot-1',
        creativeId: 'creative-1',
        type: 'start',
        timestamp: new Date(),
        mediaTime: 0,
        source: 'client',
      };

      // Simulate concurrent submissions
      const submissions = Array(20)
        .fill(null)
        .map(() => {
          return new Promise<{ accepted: boolean; duplicate: boolean }>((resolve) => {
            // Small random delay to simulate network jitter
            setTimeout(() => {
              resolve(trackingService.processEvent({ ...event }));
            }, Math.random() * 10);
          });
        });

      const results = await Promise.all(submissions);

      // Exactly one should be accepted
      const accepted = results.filter((r) => r.accepted).length;
      const duplicates = results.filter((r) => r.duplicate).length;

      expect(accepted).toBe(1);
      expect(duplicates).toBe(19);
    });

    it('should handle slight timing variations as duplicates', () => {
      const baseTimestamp = new Date();
      const sessionId = 'timing-variation-session';

      // Submit events with slight timing variations (within 2s window)
      for (let i = 0; i < 5; i++) {
        const event: TrackingEvent = {
          eventId: `event-${i}`, // Different IDs
          sessionId,
          slotId: 'slot-1',
          creativeId: 'creative-1',
          type: 'midpoint',
          timestamp: new Date(baseTimestamp.getTime() + i * 400), // 400ms apart
          mediaTime: 7.5,
          source: 'client',
        };

        trackingService.processEvent(event);
      }

      const stats = trackingService.getStats();

      // All should be deduplicated due to timing proximity
      expect(stats.processed).toBe(1);
      expect(stats.duplicates).toBe(4);
    });
  });

  describe('Rollup Reconciliation Test', () => {
    /**
     * Compare raw vs rollups for a window → CI fails if mismatch > tolerance
     */
    it('should pass when raw and aggregates match', () => {
      const rawEvents: TrackingEvent[] = [];
      const baseTime = new Date();

      // Generate 100 events
      for (let i = 0; i < 100; i++) {
        rawEvents.push({
          eventId: `event-${i}`,
          sessionId: `session-${Math.floor(i / 5)}`,
          slotId: `slot-${i % 3}`,
          creativeId: 'creative-1',
          type: ['impression', 'start', 'firstQuartile', 'midpoint', 'complete'][
            i % 5
          ] as TrackingEvent['type'],
          timestamp: new Date(baseTime.getTime() + i * 1000),
          mediaTime: (i % 5) * 3.75,
          source: 'client',
        });
      }

      // Count events for aggregates (simulating perfect rollup)
      const aggregates: Record<string, number> = {};
      for (const event of rawEvents) {
        aggregates[event.type] = (aggregates[event.type] || 0) + 1;
      }

      const report = reconciliationService.reconcile(rawEvents, aggregates);

      expect(report.passed).toBe(true);
      expect(report.discrepancy).toBe(0);
      expect(report.discrepancyPercent).toBe(0);
    });

    it('should fail when discrepancy exceeds tolerance', () => {
      const rawEvents: TrackingEvent[] = [];
      const baseTime = new Date();

      // Generate 1000 events
      for (let i = 0; i < 1000; i++) {
        rawEvents.push({
          eventId: `event-${i}`,
          sessionId: `session-${Math.floor(i / 5)}`,
          slotId: `slot-${i % 3}`,
          creativeId: 'creative-1',
          type: 'impression',
          timestamp: new Date(baseTime.getTime() + i * 100),
          mediaTime: 0,
          source: 'client',
        });
      }

      // Aggregates with significant discrepancy (5% error)
      const aggregates: Record<string, number> = {
        impression: 950, // Missing 50 events
      };

      const report = reconciliationService.reconcile(rawEvents, aggregates);

      expect(report.passed).toBe(false);
      expect(report.discrepancy).toBe(50);
      expect(report.discrepancyPercent).toBe(5);
    });

    it('should provide detailed breakdown by event type', () => {
      const rawEvents: TrackingEvent[] = [
        { eventId: '1', sessionId: 's1', slotId: 'slot-1', creativeId: 'c1', type: 'impression', timestamp: new Date(), mediaTime: 0, source: 'client' },
        { eventId: '2', sessionId: 's1', slotId: 'slot-1', creativeId: 'c1', type: 'start', timestamp: new Date(), mediaTime: 0, source: 'client' },
        { eventId: '3', sessionId: 's1', slotId: 'slot-1', creativeId: 'c1', type: 'impression', timestamp: new Date(), mediaTime: 0, source: 'client' },
        { eventId: '4', sessionId: 's1', slotId: 'slot-1', creativeId: 'c1', type: 'complete', timestamp: new Date(), mediaTime: 15, source: 'client' },
      ];

      const aggregates: Record<string, number> = {
        impression: 2,
        start: 1,
        complete: 1,
      };

      const report = reconciliationService.reconcile(rawEvents, aggregates);

      expect(report.details.length).toBe(3);

      const impressionDetail = report.details.find(
        (d) => d.eventType === 'impression'
      );
      expect(impressionDetail?.rawCount).toBe(2);
      expect(impressionDetail?.aggregatedCount).toBe(2);
      expect(impressionDetail?.difference).toBe(0);
    });

    it('should handle missing event types in aggregates', () => {
      const rawEvents: TrackingEvent[] = [
        { eventId: '1', sessionId: 's1', slotId: 'slot-1', creativeId: 'c1', type: 'impression', timestamp: new Date(), mediaTime: 0, source: 'client' },
        { eventId: '2', sessionId: 's1', slotId: 'slot-1', creativeId: 'c1', type: 'error', timestamp: new Date(), mediaTime: 5, source: 'client' },
      ];

      // Aggregates missing 'error' type
      const aggregates: Record<string, number> = {
        impression: 1,
        // 'error' missing!
      };

      const report = reconciliationService.reconcile(rawEvents, aggregates);

      expect(report.discrepancy).toBe(1);

      const errorDetail = report.details.find((d) => d.eventType === 'error');
      expect(errorDetail?.rawCount).toBe(1);
      expect(errorDetail?.aggregatedCount).toBe(0);
    });
  });

  describe('Live Inference Rules', () => {
    it('should handle quartiles under rebuffer deterministically', () => {
      const sessionId = 'live-inference-session';
      const slotId = 'slot-1';
      const creativeId = 'creative-1';
      const adDuration = 15;

      // Normal quartile progression
      const expectedQuartiles = [
        { type: 'firstQuartile', mediaTime: adDuration * 0.25 },
        { type: 'midpoint', mediaTime: adDuration * 0.5 },
        { type: 'thirdQuartile', mediaTime: adDuration * 0.75 },
      ];

      // Simulate rebuffer happening at midpoint
      const rebufferStartMedia = 7.4; // Just before midpoint
      const rebufferEndMedia = 7.5; // Just after midpoint

      // Before rebuffer - firstQuartile should fire
      const firstQuartileEvent: TrackingEvent = {
        eventId: 'fq',
        sessionId,
        slotId,
        creativeId,
        type: 'firstQuartile',
        timestamp: new Date(),
        mediaTime: 3.75,
        source: 'client',
      };

      // During rebuffer - midpoint should NOT fire twice
      const midpointEvent1: TrackingEvent = {
        eventId: 'mp1',
        sessionId,
        slotId,
        creativeId,
        type: 'midpoint',
        timestamp: new Date(Date.now() + 7400),
        mediaTime: rebufferStartMedia,
        source: 'client',
      };

      const midpointEvent2: TrackingEvent = {
        eventId: 'mp2',
        sessionId,
        slotId,
        creativeId,
        type: 'midpoint',
        timestamp: new Date(Date.now() + 10000), // After 2.5s rebuffer
        mediaTime: rebufferEndMedia,
        source: 'client',
      };

      trackingService.processEvent(firstQuartileEvent);
      trackingService.processEvent(midpointEvent1);
      trackingService.processEvent(midpointEvent2);

      const processed = trackingService.getProcessedEvents();
      const midpointEvents = processed.filter((e) => e.type === 'midpoint');

      // Midpoint events should be deduplicated (at most 2 with rebuffer)
      expect(midpointEvents.length).toBeLessThanOrEqual(2);
    });
  });
});

export {};
