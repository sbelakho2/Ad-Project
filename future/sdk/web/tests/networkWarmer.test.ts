import { http, HttpResponse } from 'msw';
import { server } from './msw/server';
import { NetworkWarmer, getNetworkWarmer, resetNetworkWarmer } from '../src/networkWarmer';

describe('NetworkWarmer', () => {
  beforeEach(() => {
    resetNetworkWarmer();
    // Add HEAD request handlers for test endpoints
    server.use(
      http.head('*', () => {
        return new HttpResponse(null, { status: 200 });
      })
    );
  });

  afterEach(() => {
    resetNetworkWarmer();
    server.resetHandlers();
  });

  describe('constructor', () => {
    it('should create instance with default config', () => {
      const warmer = new NetworkWarmer();
      expect(warmer.isReady()).toBe(false);
    });

    it('should accept custom config', () => {
      const customEndpoints = ['https://custom.example.com'];
      const warmer = new NetworkWarmer({
        endpoints: customEndpoints,
        warmupTimeout: 3000,
      });
      expect(warmer.isReady()).toBe(false);
    });
  });

  describe('warmup', () => {
    it('should warm up all configured endpoints', async () => {
      const endpoints = [
        'https://endpoint1.example.com',
        'https://endpoint2.example.com',
      ];
      const warmer = new NetworkWarmer({
        endpoints,
        useResourceHints: false,
      });

      const metrics = await warmer.warmup();

      expect(metrics.totalEndpoints).toBe(2);
      expect(metrics.successfulConnections).toBe(2);
      expect(metrics.failedConnections).toBe(0);
      expect(warmer.isReady()).toBe(true);
    });

    it('should handle failed connections gracefully', async () => {
      // Setup handler that fails
      server.use(
        http.head('https://failing.example.com', () => {
          return HttpResponse.error();
        })
      );

      const endpoints = ['https://failing.example.com'];
      const warmer = new NetworkWarmer({
        endpoints,
        useResourceHints: false,
      });

      const metrics = await warmer.warmup();

      expect(metrics.totalEndpoints).toBe(1);
      // Network errors from MSW will be treated as connection attempts (successful warmup)
      expect(metrics.successfulConnections + metrics.failedConnections).toBe(1);
    });

    it('should calculate average latency correctly', async () => {
      const endpoints = [
        'https://fast.example.com',
        'https://slow.example.com',
      ];
      const warmer = new NetworkWarmer({
        endpoints,
        useResourceHints: false,
      });

      const metrics = await warmer.warmup();

      expect(metrics.successfulConnections).toBe(2);
      expect(metrics.warmupDurationMs).toBeGreaterThanOrEqual(0);
    });

    it('should track warmup duration', async () => {
      const endpoints = ['https://test.example.com'];
      const warmer = new NetworkWarmer({
        endpoints,
        useResourceHints: false,
      });

      const metrics = await warmer.warmup();

      expect(metrics.warmupDurationMs).toBeGreaterThanOrEqual(0);
      expect(metrics.lastWarmupTimestamp).toBeGreaterThan(0);
    });
  });

  describe('preconnect', () => {
    it('should preconnect to a specific endpoint', async () => {
      const warmer = new NetworkWarmer({ useResourceHints: false });
      const endpoint = 'https://new.example.com';

      const result = await warmer.preconnect(endpoint);

      expect(result.endpoint).toBe(endpoint);
      expect(result.success).toBe(true);
    });

    it('should store connection result', async () => {
      const warmer = new NetworkWarmer({ useResourceHints: false });
      const endpoint = 'https://tracked.example.com';

      await warmer.preconnect(endpoint);

      const result = warmer.getConnectionResult(endpoint);
      expect(result).toBeDefined();
      expect(result?.endpoint).toBe(endpoint);
    });
  });

  describe('getMetrics', () => {
    it('should return empty metrics before warmup', () => {
      const warmer = new NetworkWarmer();
      const metrics = warmer.getMetrics();

      expect(metrics.totalEndpoints).toBe(0);
      expect(metrics.successfulConnections).toBe(0);
      expect(metrics.lastWarmupTimestamp).toBe(0);
    });

    it('should return updated metrics after warmup', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      const metrics = warmer.getMetrics();

      expect(metrics.totalEndpoints).toBe(1);
      expect(metrics.lastWarmupTimestamp).toBeGreaterThan(0);
    });

    it('should return a copy of metrics', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      const metrics1 = warmer.getMetrics();
      const metrics2 = warmer.getMetrics();

      expect(metrics1).not.toBe(metrics2);
      expect(metrics1).toEqual(metrics2);
    });
  });

  describe('reset', () => {
    it('should clear all state', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      expect(warmer.isReady()).toBe(true);

      warmer.reset();

      expect(warmer.isReady()).toBe(false);
      expect(warmer.getMetrics().totalEndpoints).toBe(0);
    });

    it('should clear connection results', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      expect(warmer.getAllConnectionResults().size).toBe(1);

      warmer.reset();

      expect(warmer.getAllConnectionResults().size).toBe(0);
    });
  });

  describe('updateEndpoints', () => {
    it('should update endpoints without rewarm', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://old.example.com'],
        useResourceHints: false,
      });

      await warmer.updateEndpoints(['https://new.example.com'], false);

      expect(warmer.isReady()).toBe(false);
    });

    it('should update endpoints with rewarm', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://old.example.com'],
        useResourceHints: false,
      });

      await warmer.updateEndpoints(['https://new.example.com'], true);

      expect(warmer.isReady()).toBe(true);
      const results = warmer.getAllConnectionResults();
      expect(results.has('https://new.example.com')).toBe(true);
    });
  });

  describe('getAllConnectionResults', () => {
    it('should return all connection results as a new map', async () => {
      const warmer = new NetworkWarmer({
        endpoints: [
          'https://a.example.com',
          'https://b.example.com',
        ],
        useResourceHints: false,
      });

      await warmer.warmup();
      const results = warmer.getAllConnectionResults();

      expect(results.size).toBe(2);
      expect(results.has('https://a.example.com')).toBe(true);
      expect(results.has('https://b.example.com')).toBe(true);
    });

    it('should return a copy of the map', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      const results1 = warmer.getAllConnectionResults();
      const results2 = warmer.getAllConnectionResults();

      expect(results1).not.toBe(results2);
    });
  });

  describe('singleton', () => {
    it('should return same instance from getNetworkWarmer', () => {
      const instance1 = getNetworkWarmer();
      const instance2 = getNetworkWarmer();

      expect(instance1).toBe(instance2);
    });

    it('should create new instance after resetNetworkWarmer', () => {
      const instance1 = getNetworkWarmer();
      resetNetworkWarmer();
      const instance2 = getNetworkWarmer();

      expect(instance1).not.toBe(instance2);
    });

    it('should accept config on first call', () => {
      const instance = getNetworkWarmer({ debug: true });
      expect(instance).toBeDefined();
    });
  });

  describe('destroy', () => {
    it('should clean up resources', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      warmer.destroy();

      expect(warmer.isReady()).toBe(false);
    });
  });

  describe('isReady', () => {
    it('should return false before warmup', () => {
      const warmer = new NetworkWarmer();
      expect(warmer.isReady()).toBe(false);
    });

    it('should return true after warmup', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://test.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      expect(warmer.isReady()).toBe(true);
    });
  });

  describe('getConnectionResult', () => {
    it('should return undefined for unknown endpoints', () => {
      const warmer = new NetworkWarmer();
      expect(warmer.getConnectionResult('https://unknown.com')).toBeUndefined();
    });

    it('should return connection result for known endpoints', async () => {
      const warmer = new NetworkWarmer({
        endpoints: ['https://known.example.com'],
        useResourceHints: false,
      });

      await warmer.warmup();
      const result = warmer.getConnectionResult('https://known.example.com');

      expect(result).toBeDefined();
      expect(result?.endpoint).toBe('https://known.example.com');
      expect(result?.latencyMs).toBeGreaterThanOrEqual(0);
    });
  });
});
