import { describe, it, expect, beforeEach, afterEach, jest } from '@jest/globals';
import { 
  EventTracker, 
  TrackingResult, 
  QuartileProgress 
} from '../../../../src/services/ssai/tracking/EventTracker';
import { DedupeLayer } from '../../../../src/services/ssai/tracking/DedupeLayer';
import { VODReconciler } from '../../../../src/services/ssai/tracking/VODReconciler';
import { LiveReconciler } from '../../../../src/services/ssai/tracking/LiveReconciler';
import { AuditBundleGenerator } from '../../../../src/services/ssai/tracking/AuditBundleGenerator';

// Mock fetch
const mockFetch = jest.fn();
global.fetch = mockFetch as any;

describe('SSAI Tracking Module', () => {
  describe('EventTracker', () => {
    let tracker: EventTracker;

    beforeEach(() => {
      tracker = new EventTracker({
        requestTimeout: 1000,
        maxRetries: 2,
        parallelRequests: 3,
      });
      jest.clearAllMocks();
    });

    describe('firePixel', () => {
      it('should fire tracking pixel successfully', async () => {
        mockFetch.mockResolvedValueOnce({ ok: true });

        const result = await tracker.firePixel('https://track.example.com/pixel?id=123');

        expect(result.success).toBe(true);
        expect(mockFetch).toHaveBeenCalledWith(
          'https://track.example.com/pixel?id=123',
          expect.any(Object)
        );
      });

      it('should expand macros in pixel URLs', async () => {
        mockFetch.mockResolvedValueOnce({ ok: true });

        const macros = {
          TIMESTAMP: '1234567890',
          CACHE_BUSTER: 'abc123',
        };

        await tracker.firePixel(
          'https://track.example.com/pixel?ts=[TIMESTAMP]&cb=[CACHE_BUSTER]',
          macros
        );

        expect(mockFetch).toHaveBeenCalledWith(
          'https://track.example.com/pixel?ts=1234567890&cb=abc123',
          expect.any(Object)
        );
      });

      it('should retry on failure', async () => {
        mockFetch
          .mockRejectedValueOnce(new Error('Network error'))
          .mockResolvedValueOnce({ ok: true });

        const result = await tracker.firePixel('https://track.example.com/pixel');

        expect(result.success).toBe(true);
        expect(mockFetch).toHaveBeenCalledTimes(2);
      });

      it('should fail after max retries', async () => {
        mockFetch.mockRejectedValue(new Error('Network error'));

        const result = await tracker.firePixel('https://track.example.com/pixel');

        expect(result.success).toBe(false);
        expect(mockFetch).toHaveBeenCalledTimes(3); // initial + 2 retries
      });
    });

    describe('firePixelBatch', () => {
      it('should fire multiple pixels in parallel', async () => {
        mockFetch.mockResolvedValue({ ok: true });

        const urls = [
          'https://track1.example.com/pixel',
          'https://track2.example.com/pixel',
          'https://track3.example.com/pixel',
        ];

        const results = await tracker.firePixelBatch(urls);

        expect(results).toHaveLength(3);
        expect(results.every(r => r.success)).toBe(true);
      });

      it('should handle partial failures', async () => {
        mockFetch
          .mockResolvedValueOnce({ ok: true })
          .mockRejectedValueOnce(new Error('Failed'))
          .mockResolvedValueOnce({ ok: true });

        const urls = [
          'https://track1.example.com/pixel',
          'https://track2.example.com/pixel',
          'https://track3.example.com/pixel',
        ];

        const results = await tracker.firePixelBatch(urls);

        const successes = results.filter(r => r.success);
        expect(successes.length).toBeGreaterThanOrEqual(2);
      });
    });

    describe('calculateQuartile', () => {
      it('should calculate correct quartile for progress', () => {
        expect(tracker.calculateQuartile(0)).toBe('start');
        expect(tracker.calculateQuartile(10)).toBe('start');
        expect(tracker.calculateQuartile(25)).toBe('firstQuartile');
        expect(tracker.calculateQuartile(40)).toBe('firstQuartile');
        expect(tracker.calculateQuartile(50)).toBe('midpoint');
        expect(tracker.calculateQuartile(60)).toBe('midpoint');
        expect(tracker.calculateQuartile(75)).toBe('thirdQuartile');
        expect(tracker.calculateQuartile(90)).toBe('thirdQuartile');
        expect(tracker.calculateQuartile(100)).toBe('complete');
      });
    });
  });

  describe('DedupeLayer', () => {
    let deduper: DedupeLayer;

    beforeEach(() => {
      deduper = new DedupeLayer({
        ttlMs: 60000,
        maxSize: 1000,
      });
    });

    describe('isDuplicate', () => {
      it('should detect duplicate events', () => {
        const eventKey = 'impression:pod-1:slot-1:creative-1';

        expect(deduper.isDuplicate(eventKey)).toBe(false);
        deduper.markAsFired(eventKey);
        expect(deduper.isDuplicate(eventKey)).toBe(true);
      });

      it('should allow different events', () => {
        deduper.markAsFired('impression:pod-1:slot-1:creative-1');
        
        expect(deduper.isDuplicate('start:pod-1:slot-1:creative-1')).toBe(false);
        expect(deduper.isDuplicate('impression:pod-1:slot-2:creative-1')).toBe(false);
      });

      it('should clear events for a slot', () => {
        const eventKey = 'impression:pod-1:slot-1:creative-1';
        deduper.markAsFired(eventKey);
        
        expect(deduper.isDuplicate(eventKey)).toBe(true);
        
        deduper.clearSlot('slot-1');
        
        expect(deduper.isDuplicate(eventKey)).toBe(false);
      });
    });

    describe('CrossSourceReconciler', () => {
      it('should reconcile client and server events', () => {
        const clientEvents = [
          { type: 'impression', slotId: 'slot-1', timestamp: 1000 },
          { type: 'start', slotId: 'slot-1', timestamp: 1100 },
          { type: 'firstQuartile', slotId: 'slot-1', timestamp: 1500 },
        ];

        const serverEvents = [
          { type: 'impression', slotId: 'slot-1', timestamp: 1005 },
          { type: 'start', slotId: 'slot-1', timestamp: 1098 },
        ];

        const result = deduper.reconcile(clientEvents as any, serverEvents as any);

        expect(result.matched).toHaveLength(2);
        expect(result.clientOnly).toHaveLength(1);
        expect(result.serverOnly).toHaveLength(0);
      });
    });
  });

  describe('VODReconciler', () => {
    let reconciler: VODReconciler;

    beforeEach(() => {
      reconciler = new VODReconciler({
        toleranceMs: 500,
      });
    });

    describe('calculateExpectedQuartiles', () => {
      it('should calculate expected quartile times for an ad', () => {
        const adDuration = 30; // 30 seconds
        const quartiles = reconciler.calculateExpectedQuartiles(adDuration);

        expect(quartiles).toEqual({
          start: 0,
          firstQuartile: 7.5,
          midpoint: 15,
          thirdQuartile: 22.5,
          complete: 30,
        });
      });
    });

    describe('reconcilePlayback', () => {
      it('should detect missing quartile events', () => {
        const playback = {
          adDuration: 30,
          currentTime: 20, // Past midpoint
          firedEvents: ['impression', 'start', 'firstQuartile'], // Missing midpoint
        };

        const result = reconciler.reconcilePlayback(playback as any);

        expect(result.missingEvents).toContain('midpoint');
      });

      it('should handle seek events correctly', () => {
        const playback = {
          adDuration: 30,
          currentTime: 25,
          firedEvents: ['impression', 'start'],
          seekEvents: [{ from: 5, to: 25 }],
        };

        const result = reconciler.reconcilePlayback(playback as any);

        // Should not flag skipped quartiles as errors due to seek
        expect(result.seekSkipped).toContain('firstQuartile');
        expect(result.seekSkipped).toContain('midpoint');
      });
    });
  });

  describe('LiveReconciler', () => {
    let reconciler: LiveReconciler;

    beforeEach(() => {
      reconciler = new LiveReconciler({
        driftToleranceMs: 2000,
        gracePeriodMs: 5000,
      });
    });

    describe('updateStatus', () => {
      it('should track live playback status', () => {
        reconciler.updateStatus('session-1', {
          currentTime: 100,
          liveEdge: 102,
          bufferLength: 5,
        });

        const status = reconciler.getStatus('session-1');

        expect(status.drift).toBe(2);
        expect(status.isWithinTolerance).toBe(true);
      });

      it('should detect excessive drift', () => {
        reconciler.updateStatus('session-1', {
          currentTime: 100,
          liveEdge: 110,
          bufferLength: 5,
        });

        const status = reconciler.getStatus('session-1');

        expect(status.drift).toBe(10);
        expect(status.isWithinTolerance).toBe(false);
      });
    });

    describe('shouldFireEvent', () => {
      it('should allow events within grace period', () => {
        const result = reconciler.shouldFireEvent('impression', {
          expectedTime: 1000,
          actualTime: 1003,
        });

        expect(result.shouldFire).toBe(true);
      });

      it('should reject events outside grace period', () => {
        const result = reconciler.shouldFireEvent('impression', {
          expectedTime: 1000,
          actualTime: 1010,
        });

        expect(result.shouldFire).toBe(false);
        expect(result.reason).toContain('grace period');
      });
    });
  });

  describe('AuditBundleGenerator', () => {
    let generator: AuditBundleGenerator;

    beforeEach(() => {
      generator = new AuditBundleGenerator({
        secretKey: 'test-secret-key',
        includeRaw: true,
      });
    });

    describe('generateBundle', () => {
      it('should generate audit bundle with signature', () => {
        const events = [
          { type: 'impression', slotId: 'slot-1', timestamp: Date.now() },
          { type: 'complete', slotId: 'slot-1', timestamp: Date.now() + 30000 },
        ];

        const bundle = generator.generateBundle('session-1', 'pod-1', events as any);

        expect(bundle.sessionId).toBe('session-1');
        expect(bundle.podId).toBe('pod-1');
        expect(bundle.events).toHaveLength(2);
        expect(bundle.signature).toBeDefined();
        expect(bundle.signature).toMatch(/^[a-f0-9]{64}$/); // SHA-256 hex
      });

      it('should include summary statistics', () => {
        const events = [
          { type: 'impression', slotId: 'slot-1', timestamp: 1000 },
          { type: 'start', slotId: 'slot-1', timestamp: 1000 },
          { type: 'complete', slotId: 'slot-1', timestamp: 31000 },
          { type: 'impression', slotId: 'slot-2', timestamp: 31000 },
          { type: 'skip', slotId: 'slot-2', timestamp: 36000 },
        ];

        const bundle = generator.generateBundle('session-1', 'pod-1', events as any);

        expect(bundle.summary.impressions).toBe(2);
        expect(bundle.summary.completions).toBe(1);
        expect(bundle.summary.skips).toBe(1);
      });

      it('should verify bundle signature', () => {
        const events = [
          { type: 'impression', slotId: 'slot-1', timestamp: Date.now() },
        ];

        const bundle = generator.generateBundle('session-1', 'pod-1', events as any);
        const isValid = generator.verifyBundle(bundle);

        expect(isValid).toBe(true);
      });

      it('should detect tampered bundles', () => {
        const events = [
          { type: 'impression', slotId: 'slot-1', timestamp: Date.now() },
        ];

        const bundle = generator.generateBundle('session-1', 'pod-1', events as any);
        
        // Tamper with the bundle
        bundle.events.push({ type: 'complete', slotId: 'slot-1', timestamp: Date.now() } as any);
        
        const isValid = generator.verifyBundle(bundle);

        expect(isValid).toBe(false);
      });
    });
  });
});
