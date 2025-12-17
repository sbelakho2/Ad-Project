/**
 * @file Mediation Debugger Tests
 * @description Comprehensive tests for mediation debugging with sanitized output
 */

import {
  MediationDebugger,
  MediationEvent,
  MediationEventType,
  DebugSession,
  DebugSnapshot,
  DebuggerConfig,
} from '../src/mediationDebugger';

describe('MediationDebugger', () => {
  let debugger_: MediationDebugger;

  beforeEach(() => {
    debugger_ = new MediationDebugger();
  });

  afterEach(() => {
    debugger_.stop();
  });

  describe('Session Management', () => {
    it('should create a new debug session', () => {
      const session = debugger_.startSession('placement-1');
      expect(session).toBeDefined();
      expect(session.id).toBeTruthy();
      expect(session.placementId).toBe('placement-1');
      expect(session.startTime).toBeLessThanOrEqual(Date.now());
      expect(session.events).toHaveLength(0);
    });

    it('should generate unique session IDs', () => {
      const session1 = debugger_.startSession('placement-1');
      const session2 = debugger_.startSession('placement-2');
      expect(session1.id).not.toBe(session2.id);
    });

    it('should stop a debug session', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.stopSession(session.id);
      const stopped = debugger_.getSession(session.id);
      expect(stopped).toBeNull();
    });

    it('should get active session for placement', () => {
      const session = debugger_.startSession('placement-1');
      const retrieved = debugger_.getSessionForPlacement('placement-1');
      expect(retrieved).not.toBeNull();
      expect(retrieved?.id).toBe(session.id);
    });

    it('should return null for inactive placement', () => {
      const session = debugger_.getSessionForPlacement('unknown');
      expect(session).toBeNull();
    });
  });

  describe('Event Recording', () => {
    it('should record adapter request events', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
      });

      const events = debugger_.getEvents(session.id);
      expect(events).toHaveLength(1);
      expect(events[0].type).toBe(MediationEventType.ADAPTER_REQUEST);
      expect(events[0].adapterId).toBe('admob');
    });

    it('should record adapter response events', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_RESPONSE,
        adapterId: 'admob',
        timestamp: Date.now(),
        latency: 150,
        success: true,
      });

      const events = debugger_.getEvents(session.id);
      expect(events[0].latency).toBe(150);
      expect(events[0].success).toBe(true);
    });

    it('should record no-fill events', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.NO_FILL,
        adapterId: 'unity',
        timestamp: Date.now(),
      });

      const events = debugger_.getEvents(session.id);
      expect(events[0].type).toBe(MediationEventType.NO_FILL);
    });

    it('should record timeout events', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.TIMEOUT,
        adapterId: 'facebook',
        timestamp: Date.now(),
        timeout: 5000,
      });

      const events = debugger_.getEvents(session.id);
      expect(events[0].type).toBe(MediationEventType.TIMEOUT);
      expect(events[0].timeout).toBe(5000);
    });

    it('should record error events', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ERROR,
        adapterId: 'vungle',
        timestamp: Date.now(),
        error: 'Network error',
      });

      const events = debugger_.getEvents(session.id);
      expect(events[0].type).toBe(MediationEventType.ERROR);
      expect(events[0].error).toBe('Network error');
    });

    it('should maintain event order', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_START,
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: baseTime + 10,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_RESPONSE,
        adapterId: 'admob',
        timestamp: baseTime + 150,
        success: true,
      });

      const events = debugger_.getEvents(session.id);
      expect(events[0].type).toBe(MediationEventType.WATERFALL_START);
      expect(events[1].type).toBe(MediationEventType.ADAPTER_REQUEST);
      expect(events[2].type).toBe(MediationEventType.ADAPTER_RESPONSE);
    });
  });

  describe('Data Sanitization', () => {
    it('should sanitize user IDs', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
        metadata: {
          userId: 'user-12345-sensitive',
          deviceId: 'device-abc-sensitive',
        },
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: true });
      expect(snapshot).not.toBeNull();
      const event = snapshot!.events[0];
      expect(event.metadata?.userId).toMatch(/^user-\*+$/);
      expect(event.metadata?.deviceId).toMatch(/^device-\*+$/);
    });

    it('should sanitize bid values by default', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.BID_RECEIVED,
        adapterId: 'admob',
        timestamp: Date.now(),
        bidAmount: 2.5,
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: true });
      const event = snapshot!.events[0];
      expect(event.bidAmount).toBe('[REDACTED]');
    });

    it('should allow raw bid values when sanitization disabled', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.BID_RECEIVED,
        adapterId: 'admob',
        timestamp: Date.now(),
        bidAmount: 2.5,
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: false });
      const event = snapshot!.events[0];
      expect(event.bidAmount).toBe(2.5);
    });

    it('should sanitize IP addresses', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
        metadata: {
          clientIp: '192.168.1.100',
        },
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: true });
      const event = snapshot!.events[0];
      expect(event.metadata?.clientIp).toBe('***.***.***.***');
    });

    it('should sanitize email addresses', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
        metadata: {
          email: 'user@example.com',
        },
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: true });
      const event = snapshot!.events[0];
      expect(event.metadata?.email).toBe('[EMAIL REDACTED]');
    });

    it('should preserve non-sensitive data', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
        metadata: {
          adFormat: 'banner',
          adSize: '320x50',
        },
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: true });
      const event = snapshot!.events[0];
      expect(event.metadata?.adFormat).toBe('banner');
      expect(event.metadata?.adSize).toBe('320x50');
    });
  });

  describe('Waterfall Visualization', () => {
    it('should calculate waterfall timeline', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_START,
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: baseTime + 5,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.NO_FILL,
        adapterId: 'admob',
        timestamp: baseTime + 100,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'unity',
        timestamp: baseTime + 105,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_RESPONSE,
        adapterId: 'unity',
        timestamp: baseTime + 200,
        success: true,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_END,
        timestamp: baseTime + 210,
      });

      const timeline = debugger_.getWaterfallTimeline(session.id);
      expect(timeline).not.toBeNull();
      expect(timeline!.totalDuration).toBe(210);
      expect(timeline!.adapters).toHaveLength(2);
      expect(timeline!.adapters[0].adapterId).toBe('admob');
      expect(timeline!.adapters[0].duration).toBe(95);
      expect(timeline!.adapters[0].result).toBe('no-fill');
      expect(timeline!.adapters[1].adapterId).toBe('unity');
      expect(timeline!.adapters[1].result).toBe('success');
    });

    it('should identify the winning adapter', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_START,
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_RESPONSE,
        adapterId: 'admob',
        timestamp: baseTime + 50,
        success: true,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.AD_RENDERED,
        adapterId: 'admob',
        timestamp: baseTime + 60,
      });

      const timeline = debugger_.getWaterfallTimeline(session.id);
      expect(timeline!.winningAdapter).toBe('admob');
    });

    it('should handle waterfall with all no-fills', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_START,
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.NO_FILL,
        adapterId: 'admob',
        timestamp: baseTime + 100,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'unity',
        timestamp: baseTime + 100,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.NO_FILL,
        adapterId: 'unity',
        timestamp: baseTime + 200,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_END,
        timestamp: baseTime + 200,
      });

      const timeline = debugger_.getWaterfallTimeline(session.id);
      expect(timeline!.winningAdapter).toBeNull();
      expect(timeline!.adapters.every((a: { result: string }) => a.result === 'no-fill')).toBe(true);
    });
  });

  describe('Performance Analysis', () => {
    it('should calculate latency statistics', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      // Record multiple adapter responses with varying latencies
      const latencies = [50, 100, 150, 200, 250, 300, 350];
      latencies.forEach((latency, i) => {
        debugger_.recordEvent(session.id, {
          type: MediationEventType.ADAPTER_RESPONSE,
          adapterId: 'admob',
          timestamp: baseTime + latency,
          latency,
          success: true,
        });
      });

      const stats = debugger_.getPerformanceStats(session.id, 'admob');
      expect(stats).not.toBeNull();
      expect(stats!.avgLatency).toBe(200); // (50+100+150+200+250+300+350) / 7
      expect(stats!.minLatency).toBe(50);
      expect(stats!.maxLatency).toBe(350);
    });

    it('should calculate success rate', () => {
      const session = debugger_.startSession('placement-1');

      // 7 successes, 3 failures
      for (let i = 0; i < 10; i++) {
        debugger_.recordEvent(session.id, {
          type: MediationEventType.ADAPTER_RESPONSE,
          adapterId: 'admob',
          timestamp: Date.now() + i,
          success: i < 7,
        });
      }

      const stats = debugger_.getPerformanceStats(session.id, 'admob');
      expect(stats!.successRate).toBe(0.7);
    });

    it('should count timeouts', () => {
      const session = debugger_.startSession('placement-1');

      debugger_.recordEvent(session.id, {
        type: MediationEventType.TIMEOUT,
        adapterId: 'admob',
        timestamp: Date.now(),
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.TIMEOUT,
        adapterId: 'admob',
        timestamp: Date.now() + 1,
      });

      const stats = debugger_.getPerformanceStats(session.id, 'admob');
      expect(stats!.timeoutCount).toBe(2);
    });
  });

  describe('Debug Output Formats', () => {
    it('should export to JSON format', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
      });

      const json = debugger_.exportToJSON(session.id);
      expect(json).toBeTruthy();
      const parsed = JSON.parse(json!);
      expect(parsed.sessionId).toBe(session.id);
      expect(parsed.events).toHaveLength(1);
    });

    it('should generate text summary', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_START,
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_RESPONSE,
        adapterId: 'admob',
        timestamp: baseTime + 100,
        success: true,
      });

      const summary = debugger_.getTextSummary(session.id);
      expect(summary).toContain('admob');
      expect(summary).toContain('100ms');
    });

    it('should generate timeline visualization', () => {
      const session = debugger_.startSession('placement-1');
      const baseTime = Date.now();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_START,
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: baseTime,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_RESPONSE,
        adapterId: 'admob',
        timestamp: baseTime + 100,
        success: true,
      });
      debugger_.recordEvent(session.id, {
        type: MediationEventType.WATERFALL_END,
        timestamp: baseTime + 100,
      });

      const viz = debugger_.getTimelineVisualization(session.id);
      expect(viz).toContain('admob');
      expect(viz).toContain('█'); // Timeline bar character
    });
  });

  describe('Configuration', () => {
    it('should respect max events limit', () => {
      debugger_.stop();
      debugger_ = new MediationDebugger({ maxEventsPerSession: 5 });
      const session = debugger_.startSession('placement-1');

      for (let i = 0; i < 10; i++) {
        debugger_.recordEvent(session.id, {
          type: MediationEventType.ADAPTER_REQUEST,
          adapterId: `adapter-${i}`,
          timestamp: Date.now() + i,
        });
      }

      const events = debugger_.getEvents(session.id);
      expect(events).toHaveLength(5);
      // Should keep the most recent events
      expect(events[4].adapterId).toBe('adapter-9');
    });

    it('should respect session timeout', async () => {
      debugger_.stop();
      debugger_ = new MediationDebugger({ sessionTimeoutMs: 50 });
      const session = debugger_.startSession('placement-1');

      await new Promise(resolve => setTimeout(resolve, 100));

      debugger_.cleanup();
      const retrieved = debugger_.getSession(session.id);
      expect(retrieved).toBeNull();
    });

    it('should enable/disable sanitization globally', () => {
      debugger_.stop();
      debugger_ = new MediationDebugger({ sanitizeByDefault: false });
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.BID_RECEIVED,
        adapterId: 'admob',
        timestamp: Date.now(),
        bidAmount: 2.5,
      });

      const snapshot = debugger_.getSnapshot(session.id);
      const event = snapshot!.events[0];
      expect(event.bidAmount).toBe(2.5);
    });
  });

  describe('Real-time Streaming', () => {
    it('should notify subscribers of new events', async () => {
      const session = debugger_.startSession('placement-1');
      const events: MediationEvent[] = [];

      debugger_.subscribe(session.id, (event: MediationEvent) => {
        events.push(event);
      });

      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(events).toHaveLength(1);
    });

    it('should unsubscribe correctly', async () => {
      const session = debugger_.startSession('placement-1');
      const events: MediationEvent[] = [];

      const unsubscribe = debugger_.subscribe(session.id, (event: MediationEvent) => {
        events.push(event);
      });

      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
      });

      unsubscribe();

      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'unity',
        timestamp: Date.now(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(events).toHaveLength(1);
    });

    it('should support multiple subscribers', async () => {
      const session = debugger_.startSession('placement-1');
      const events1: MediationEvent[] = [];
      const events2: MediationEvent[] = [];

      debugger_.subscribe(session.id, (event: MediationEvent) => events1.push(event));
      debugger_.subscribe(session.id, (event: MediationEvent) => events2.push(event));

      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(events1).toHaveLength(1);
      expect(events2).toHaveLength(1);
    });
  });

  describe('Error Handling', () => {
    it('should handle invalid session ID gracefully', () => {
      const events = debugger_.getEvents('invalid-session');
      expect(events).toEqual([]);
    });

    it('should handle recording to stopped session', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.stopSession(session.id);

      // Should not throw
      expect(() => {
        debugger_.recordEvent(session.id, {
          type: MediationEventType.ADAPTER_REQUEST,
          adapterId: 'admob',
          timestamp: Date.now(),
        });
      }).not.toThrow();
    });

    it('should handle null metadata gracefully', () => {
      const session = debugger_.startSession('placement-1');
      debugger_.recordEvent(session.id, {
        type: MediationEventType.ADAPTER_REQUEST,
        adapterId: 'admob',
        timestamp: Date.now(),
        metadata: undefined,
      });

      const snapshot = debugger_.getSnapshot(session.id, { sanitize: true });
      expect(snapshot).not.toBeNull();
    });
  });
});
