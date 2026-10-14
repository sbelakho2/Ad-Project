/**
 * @file Usage Meter Tests
 * @description Comprehensive tests for usage metering and reporting
 */

import {
  UsageMeter,
  UsageMeterBuilder,
  UsageEventType,
  UsageMetrics,
  MeteringReporter,
} from '../src/usageMeter';

describe('UsageMeter', () => {
  let meter: UsageMeter;

  beforeEach(() => {
    meter = new UsageMeter({ enableRemoteReporting: false });
  });

  afterEach(() => {
    meter.stop();
  });

  describe('Event Recording', () => {
    it('should record ad request events', () => {
      meter.recordRequest('placement-1', 'admob');
      const metrics = meter.getMetrics();
      expect(metrics.adRequests).toBe(1);
    });

    it('should record ad impression events', () => {
      meter.recordImpression('placement-1', 'admob');
      const metrics = meter.getMetrics();
      expect(metrics.adImpressions).toBe(1);
    });

    it('should record ad click events', () => {
      meter.recordClick('placement-1', 'admob');
      const metrics = meter.getMetrics();
      expect(metrics.adClicks).toBe(1);
    });

    it('should record video start events', () => {
      meter.recordVideoStart('placement-1', 'admob');
      const metrics = meter.getMetrics();
      expect(metrics.videoStarts).toBe(1);
    });

    it('should record video complete events', () => {
      meter.recordVideoComplete('placement-1', 'admob');
      const metrics = meter.getMetrics();
      expect(metrics.videoCompletes).toBe(1);
    });

    it('should record revenue events', () => {
      meter.recordRevenue('placement-1', 'admob', 2.5);
      const metrics = meter.getMetrics();
      expect(metrics.totalRevenue).toBe(2.5);
    });

    it('should accumulate revenue', () => {
      meter.recordRevenue('placement-1', 'admob', 2.5);
      meter.recordRevenue('placement-1', 'unity', 1.5);
      meter.recordRevenue('placement-1', 'facebook', 3.0);
      const metrics = meter.getMetrics();
      expect(metrics.totalRevenue).toBe(7.0);
    });

    it('should record cache hit events', () => {
      meter.recordCacheHit('placement-1');
      const metrics = meter.getMetrics();
      expect(metrics.cacheHits).toBe(1);
    });

    it('should record cache miss events', () => {
      meter.recordCacheMiss('placement-1');
      const metrics = meter.getMetrics();
      expect(metrics.cacheMisses).toBe(1);
    });

    it('should record error events', () => {
      meter.recordError('placement-1', 'admob');
      const metrics = meter.getMetrics();
      expect(metrics.errors).toBe(1);
    });

    it('should handle raw event recording', () => {
      meter.record({
        type: UsageEventType.API_CALL,
        timestamp: Date.now(),
      });
      const metrics = meter.getMetrics();
      expect(metrics.apiCalls).toBe(1);
    });
  });

  describe('Dimensional Breakdown', () => {
    it('should track metrics by placement', () => {
      meter.recordRequest('placement-1', 'admob');
      meter.recordRequest('placement-1', 'admob');
      meter.recordRequest('placement-2', 'admob');

      const breakdown = meter.getBreakdown();
      expect(breakdown.byPlacement.get('placement-1')?.adRequests).toBe(2);
      expect(breakdown.byPlacement.get('placement-2')?.adRequests).toBe(1);
    });

    it('should track metrics by adapter', () => {
      meter.recordImpression('placement-1', 'admob');
      meter.recordImpression('placement-1', 'admob');
      meter.recordImpression('placement-1', 'unity');

      const breakdown = meter.getBreakdown();
      expect(breakdown.byAdapter.get('admob')?.adImpressions).toBe(2);
      expect(breakdown.byAdapter.get('unity')?.adImpressions).toBe(1);
    });

    it('should track metrics by ad format', () => {
      meter.recordRequest('placement-1', 'admob', 'banner');
      meter.recordRequest('placement-1', 'admob', 'banner');
      meter.recordRequest('placement-1', 'admob', 'interstitial');

      const breakdown = meter.getBreakdown();
      expect(breakdown.byAdFormat.get('banner')?.adRequests).toBe(2);
      expect(breakdown.byAdFormat.get('interstitial')?.adRequests).toBe(1);
    });

    it('should track video events with video format', () => {
      meter.recordVideoStart('placement-1', 'admob');
      meter.recordVideoComplete('placement-1', 'admob');

      const breakdown = meter.getBreakdown();
      expect(breakdown.byAdFormat.get('video')?.videoStarts).toBe(1);
      expect(breakdown.byAdFormat.get('video')?.videoCompletes).toBe(1);
    });
  });

  describe('Computed Metrics', () => {
    it('should calculate click-through rate', () => {
      // 3 impressions, 1 click = 33.33% CTR
      meter.recordImpression('p1', 'a1');
      meter.recordImpression('p1', 'a1');
      meter.recordImpression('p1', 'a1');
      meter.recordClick('p1', 'a1');

      expect(meter.getCTR()).toBeCloseTo(1 / 3, 5);
    });

    it('should return 0 CTR with no impressions', () => {
      expect(meter.getCTR()).toBe(0);
    });

    it('should calculate fill rate', () => {
      // 10 requests, 7 impressions = 70% fill
      for (let i = 0; i < 10; i++) {
        meter.recordRequest('p1', 'a1');
      }
      for (let i = 0; i < 7; i++) {
        meter.recordImpression('p1', 'a1');
      }

      expect(meter.getFillRate()).toBeCloseTo(0.7, 5);
    });

    it('should return 0 fill rate with no requests', () => {
      expect(meter.getFillRate()).toBe(0);
    });

    it('should calculate video completion rate', () => {
      // 4 starts, 3 completes = 75% completion
      meter.recordVideoStart('p1', 'a1');
      meter.recordVideoStart('p1', 'a1');
      meter.recordVideoStart('p1', 'a1');
      meter.recordVideoStart('p1', 'a1');
      meter.recordVideoComplete('p1', 'a1');
      meter.recordVideoComplete('p1', 'a1');
      meter.recordVideoComplete('p1', 'a1');

      expect(meter.getVideoCompletionRate()).toBeCloseTo(0.75, 5);
    });

    it('should return 0 video completion rate with no starts', () => {
      expect(meter.getVideoCompletionRate()).toBe(0);
    });

    it('should calculate cache hit rate', () => {
      // 8 hits, 2 misses = 80% hit rate
      for (let i = 0; i < 8; i++) {
        meter.recordCacheHit();
      }
      for (let i = 0; i < 2; i++) {
        meter.recordCacheMiss();
      }

      expect(meter.getCacheHitRate()).toBeCloseTo(0.8, 5);
    });

    it('should return 0 cache hit rate with no cache operations', () => {
      expect(meter.getCacheHitRate()).toBe(0);
    });

    it('should calculate effective CPM', () => {
      // $10 total revenue, 5000 impressions = $2 eCPM
      for (let i = 0; i < 5000; i++) {
        meter.recordImpression('p1', 'a1');
      }
      meter.recordRevenue('p1', 'a1', 10);

      expect(meter.getEffectiveCPM()).toBeCloseTo(2.0, 5);
    });

    it('should return 0 eCPM with no impressions', () => {
      meter.recordRevenue('p1', 'a1', 10);
      expect(meter.getEffectiveCPM()).toBe(0);
    });
  });

  describe('Period Tracking', () => {
    it('should track period start time', () => {
      const before = Date.now();
      meter = new UsageMeter({ enableRemoteReporting: false });
      const after = Date.now();

      const metrics = meter.getMetrics();
      expect(metrics.periodStart).toBeGreaterThanOrEqual(before);
      expect(metrics.periodStart).toBeLessThanOrEqual(after);
    });

    it('should track period end time', () => {
      meter.recordRequest('p1', 'a1');
      const before = Date.now();
      const metrics = meter.getMetrics();

      expect(metrics.periodEnd).toBeGreaterThanOrEqual(before);
      expect(metrics.periodEnd).toBeLessThanOrEqual(Date.now());
    });

    it('should reset period start on reset', () => {
      const originalStart = meter.getMetrics().periodStart;
      meter.recordRequest('p1', 'a1');

      // Wait a tiny bit
      const newMeter = new UsageMeter({ enableRemoteReporting: false });
      newMeter.reset();
      const newStart = newMeter.getMetrics().periodStart;

      expect(newStart).toBeGreaterThanOrEqual(originalStart);
      newMeter.stop();
    });
  });

  describe('Sampling', () => {
    it('should respect sampling rate', () => {
      // With 50% sampling, roughly half of events should be recorded
      meter = new UsageMeter({
        enableRemoteReporting: false,
        samplingRate: 0.5,
      });

      for (let i = 0; i < 1000; i++) {
        meter.recordRequest('p1', 'a1');
      }

      const metrics = meter.getMetrics();
      // Should be roughly 500 ± some variance
      expect(metrics.adRequests).toBeGreaterThan(300);
      expect(metrics.adRequests).toBeLessThan(700);
    });

    it('should record all events with sampling rate 1.0', () => {
      meter = new UsageMeter({
        enableRemoteReporting: false,
        samplingRate: 1.0,
      });

      for (let i = 0; i < 100; i++) {
        meter.recordRequest('p1', 'a1');
      }

      expect(meter.getMetrics().adRequests).toBe(100);
    });

    it('should record no events with sampling rate 0', () => {
      meter = new UsageMeter({
        enableRemoteReporting: false,
        samplingRate: 0,
      });

      for (let i = 0; i < 100; i++) {
        meter.recordRequest('p1', 'a1');
      }

      expect(meter.getMetrics().adRequests).toBe(0);
    });
  });

  describe('Reset', () => {
    it('should reset all global counters', () => {
      meter.recordRequest('p1', 'a1');
      meter.recordImpression('p1', 'a1');
      meter.recordClick('p1', 'a1');
      meter.recordRevenue('p1', 'a1', 5.0);
      meter.recordVideoStart('p1', 'a1');
      meter.recordVideoComplete('p1', 'a1');
      meter.recordCacheHit();
      meter.recordCacheMiss();
      meter.recordError();

      meter.reset();

      const metrics = meter.getMetrics();
      expect(metrics.adRequests).toBe(0);
      expect(metrics.adImpressions).toBe(0);
      expect(metrics.adClicks).toBe(0);
      expect(metrics.totalRevenue).toBe(0);
      expect(metrics.videoStarts).toBe(0);
      expect(metrics.videoCompletes).toBe(0);
      expect(metrics.cacheHits).toBe(0);
      expect(metrics.cacheMisses).toBe(0);
      expect(metrics.errors).toBe(0);
    });

    it('should reset dimensional breakdowns', () => {
      meter.recordRequest('p1', 'a1', 'banner');
      meter.reset();

      const breakdown = meter.getBreakdown();
      expect(breakdown.byPlacement.get('p1')?.adRequests ?? 0).toBe(0);
      expect(breakdown.byAdapter.get('a1')?.adRequests ?? 0).toBe(0);
      expect(breakdown.byAdFormat.get('banner')?.adRequests ?? 0).toBe(0);
    });
  });

  describe('Export', () => {
    it('should export metrics as JSON', () => {
      meter.recordRequest('p1', 'a1', 'banner');
      meter.recordImpression('p1', 'a1', 'banner');
      meter.recordClick('p1', 'a1', 'banner');
      meter.recordRevenue('p1', 'a1', 2.5);

      const json = meter.exportAsJSON();
      const parsed = JSON.parse(json);

      expect(parsed.metrics.adRequests).toBe(1);
      expect(parsed.metrics.adImpressions).toBe(1);
      expect(parsed.metrics.adClicks).toBe(1);
      expect(parsed.metrics.totalRevenue).toBe(2.5);
      expect(parsed.computed.ctr).toBe(1);
      expect(parsed.computed.fillRate).toBe(1);
      expect(parsed.breakdown.placementCount).toBe(1);
      expect(parsed.breakdown.adapterCount).toBe(1);
      expect(parsed.breakdown.formatCount).toBe(1);
    });

    it('should export valid JSON', () => {
      meter.recordRequest('p1', 'a1');
      const json = meter.exportAsJSON();
      expect(() => JSON.parse(json)).not.toThrow();
    });
  });

  describe('Flush and Reporting', () => {
    it('should call reporter on flush', async () => {
      const reportedMetrics: UsageMetrics[] = [];
      const reporter: MeteringReporter = {
        report: async (metrics) => {
          reportedMetrics.push(metrics);
          return true;
        },
      };

      meter = new UsageMeter({ enableRemoteReporting: true }, reporter);
      meter.recordRequest('p1', 'a1');
      meter.recordImpression('p1', 'a1');

      const success = await meter.flush();
      expect(success).toBe(true);
      expect(reportedMetrics).toHaveLength(1);
      expect(reportedMetrics[0].adRequests).toBe(1);
      expect(reportedMetrics[0].adImpressions).toBe(1);
    });

    it('should reset after successful flush', async () => {
      const reporter: MeteringReporter = {
        report: async () => true,
      };

      meter = new UsageMeter({ enableRemoteReporting: true }, reporter);
      meter.recordRequest('p1', 'a1');

      await meter.flush();

      expect(meter.getMetrics().adRequests).toBe(0);
    });

    it('should not reset after failed flush', async () => {
      const reporter: MeteringReporter = {
        report: async () => false,
      };

      meter = new UsageMeter({ enableRemoteReporting: true }, reporter);
      meter.recordRequest('p1', 'a1');

      await meter.flush();

      expect(meter.getMetrics().adRequests).toBe(1);
    });

    it('should handle reporter errors gracefully', async () => {
      const reporter: MeteringReporter = {
        report: async () => {
          throw new Error('Network error');
        },
      };

      meter = new UsageMeter({ enableRemoteReporting: true }, reporter);
      meter.recordRequest('p1', 'a1');

      const success = await meter.flush();
      expect(success).toBe(false);
      expect(meter.getMetrics().adRequests).toBe(1);
    });

    it('should skip flush when remote reporting is disabled', async () => {
      let called = false;
      const reporter: MeteringReporter = {
        report: async () => {
          called = true;
          return true;
        },
      };

      meter = new UsageMeter({ enableRemoteReporting: false }, reporter);
      meter.recordRequest('p1', 'a1');

      const success = await meter.flush();
      expect(success).toBe(true);
      expect(called).toBe(false);
    });
  });

  describe('Builder', () => {
    it('should build with default config', () => {
      const meter = new UsageMeterBuilder().build();
      meter.recordRequest('p1', 'a1');
      expect(meter.getMetrics().adRequests).toBe(1);
      meter.stop();
    });

    it('should build with custom flush interval', () => {
      const meter = new UsageMeterBuilder().withFlushInterval(5000).build();
      meter.stop();
    });

    it('should build with custom sampling rate', () => {
      const meter = new UsageMeterBuilder().withSamplingRate(0.5).build();
      meter.stop();
    });

    it('should clamp sampling rate to valid range', () => {
      const meter = new UsageMeterBuilder().withSamplingRate(2.0).build();
      // Should be clamped to 1.0
      for (let i = 0; i < 100; i++) {
        meter.recordRequest('p1', 'a1');
      }
      expect(meter.getMetrics().adRequests).toBe(100);
      meter.stop();
    });

    it('should build with custom reporter', async () => {
      let reported = false;
      const reporter: MeteringReporter = {
        report: async () => {
          reported = true;
          return true;
        },
      };

      const meter = new UsageMeterBuilder()
        .withReporter(reporter)
        .enableRemoteReporting(true)
        .build();

      meter.recordRequest('p1', 'a1');
      await meter.flush();

      expect(reported).toBe(true);
      meter.stop();
    });

    it('should chain builder methods', () => {
      const meter = new UsageMeterBuilder()
        .withFlushInterval(30000)
        .withSamplingRate(1.0)
        .enableLocalStorage(true)
        .enableRemoteReporting(false)
        .withReportEndpoint('/custom/metrics')
        .build();

      meter.recordRequest('p1', 'a1');
      expect(meter.getMetrics().adRequests).toBe(1);
      meter.stop();
    });
  });

  describe('Service Lifecycle', () => {
    it('should start and stop cleanly', () => {
      meter.start();
      expect(() => meter.stop()).not.toThrow();
    });

    it('should handle multiple start calls', () => {
      meter.start();
      meter.start();
      meter.start();
      meter.stop();
    });

    it('should handle multiple stop calls', () => {
      meter.start();
      meter.stop();
      meter.stop();
      meter.stop();
    });

    it('should record events without starting', () => {
      meter.recordRequest('p1', 'a1');
      expect(meter.getMetrics().adRequests).toBe(1);
    });
  });

  describe('Pending Events', () => {
    it('should track pending events count', () => {
      meter = new UsageMeter({ enableRemoteReporting: false, enableLocalStorage: true });

      meter.recordRequest('p1', 'a1');
      meter.recordRequest('p1', 'a1');
      meter.recordRequest('p1', 'a1');

      expect(meter.getPendingEventsCount()).toBe(3);
    });

    it('should clear pending events on reset', () => {
      meter = new UsageMeter({ enableRemoteReporting: false, enableLocalStorage: true });

      meter.recordRequest('p1', 'a1');
      meter.reset();

      expect(meter.getPendingEventsCount()).toBe(0);
    });
  });
});
