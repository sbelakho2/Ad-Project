/**
 * SSAI Tracking Module Unit Tests
 */

import { BeaconManager } from '../tracking/BeaconManager';
import { AdTrackingService } from '../tracking/AdTrackingService';

describe('BeaconManager', () => {
  let manager: BeaconManager;

  beforeEach(() => {
    manager = new BeaconManager();
  });

  describe('createBeacon', () => {
    it('should create beacon with correct type', () => {
      const beacon = manager.createBeacon({
        type: 'impression',
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        url: 'https://tracking.example.com/impression',
      });

      expect(beacon.id).toBeDefined();
      expect(beacon.type).toBe('impression');
    });

    it('should create quartile beacons', () => {
      const beacons = manager.createQuartileBeacons({
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        duration: 30,
        urls: {
          start: 'https://tracking.example.com/start',
          firstQuartile: 'https://tracking.example.com/first',
          midpoint: 'https://tracking.example.com/mid',
          thirdQuartile: 'https://tracking.example.com/third',
          complete: 'https://tracking.example.com/complete',
        },
      });

      expect(beacons.length).toBe(5);
      expect(beacons.some(b => b.type === 'start')).toBe(true);
      expect(beacons.some(b => b.type === 'complete')).toBe(true);
    });
  });

  describe('shouldFire', () => {
    it('should return true when playhead reaches beacon', () => {
      const beacon = manager.createBeacon({
        type: 'midpoint',
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        url: 'https://tracking.example.com/midpoint',
        triggerTime: 15,
      });

      const shouldFire = manager.shouldFire(beacon, 15.5);

      expect(shouldFire).toBe(true);
    });

    it('should not fire before trigger time', () => {
      const beacon = manager.createBeacon({
        type: 'complete',
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        url: 'https://tracking.example.com/complete',
        triggerTime: 30,
      });

      const shouldFire = manager.shouldFire(beacon, 10);

      expect(shouldFire).toBe(false);
    });
  });

  describe('deduplication', () => {
    it('should not fire same beacon twice', () => {
      const beacon = manager.createBeacon({
        type: 'impression',
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        url: 'https://tracking.example.com/impression',
        triggerTime: 0,
      });

      manager.markFired(beacon.id);
      const shouldFire = manager.shouldFire(beacon, 0);

      expect(shouldFire).toBe(false);
    });
  });
});

describe('AdTrackingService', () => {
  let service: AdTrackingService;

  beforeEach(() => {
    service = new AdTrackingService();
  });

  describe('recordImpression', () => {
    it('should record impression event', async () => {
      const result = await service.recordImpression({
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        timestamp: Date.now(),
      });

      expect(result.recorded).toBe(true);
    });
  });

  describe('getSessionMetrics', () => {
    it('should return session metrics', async () => {
      await service.recordImpression({
        sessionId: 'sess-1',
        podId: 'pod-1',
        creativeId: 'creative-1',
        timestamp: Date.now(),
      });

      const metrics = await service.getSessionMetrics('sess-1');

      expect(metrics.impressions).toBe(1);
    });
  });
});
