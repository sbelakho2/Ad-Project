/**
 * Tests for Shadow Fraud Scoring Service
 * SDK_CHECKS Part 7.1: Shadow fraud scoring ON; never block
 */

import { 
  scoreShadow, 
  calculatePSI, 
  extractFeatures, 
  heuristicScore,
  ShadowScoreInput 
} from '../shadowFraudScoring';

// Mock prom-client to avoid metric registration conflicts
jest.mock('prom-client', () => ({
  collectDefaultMetrics: jest.fn(),
  register: {
    metrics: jest.fn().mockResolvedValue(''),
  },
  Histogram: jest.fn().mockImplementation(() => ({
    observe: jest.fn(),
    startTimer: jest.fn(() => jest.fn()),
  })),
  Counter: jest.fn().mockImplementation(() => ({
    inc: jest.fn(),
  })),
  Gauge: jest.fn().mockImplementation(() => ({
    set: jest.fn(),
  })),
}));

// Mock postgres
jest.mock('../../../utils/postgres', () => ({
  query: jest.fn().mockResolvedValue({ rows: [] }),
}));

// Mock mlCanary
jest.mock('../../../utils/mlCanary', () => ({
  selectModelVersion: jest.fn().mockReturnValue({ version: 'stable', isCanary: false }),
  getModelEndpoint: jest.fn().mockReturnValue('http://localhost:8000/score'),
}));

// Mock logger
jest.mock('../../../utils/logger', () => ({
  default: {
    warn: jest.fn(),
    debug: jest.fn(),
    info: jest.fn(),
    error: jest.fn(),
  },
}));

// Mock fetch globally
const mockFetch = jest.fn();
global.fetch = mockFetch;

describe('Shadow Fraud Scoring', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('extractFeatures', () => {
    it('should extract device features correctly', () => {
      const input: ShadowScoreInput = {
        requestId: 'req-123',
        placementId: 'placement-abc',
        deviceInfo: {
          ip: '192.168.1.1',
          userAgent: 'Mozilla/5.0',
          platform: 'iOS',
          osVersion: '17.0',
        },
      };

      const features = extractFeatures(input);

      expect(features['has_ip']).toBe(1);
      expect(features['has_user_agent']).toBe(1);
      expect(features['is_mobile']).toBe(1);
    });

    it('should mark missing device info', () => {
      const input: ShadowScoreInput = {
        requestId: 'req-123',
        placementId: 'placement-abc',
        deviceInfo: {},
      };

      const features = extractFeatures(input);

      expect(features['has_ip']).toBe(0);
      expect(features['has_user_agent']).toBe(0);
    });

    it('should extract auction context features', () => {
      const input: ShadowScoreInput = {
        requestId: 'req-123',
        placementId: 'placement-abc',
        auctionContext: {
          floorCpm: 2.0,
          bidCount: 5,
          winningCpm: 4.0,
          latencyMs: 50,
        },
      };

      const features = extractFeatures(input);

      expect(features['floor_cpm']).toBe(2.0);
      expect(features['bid_count']).toBe(5);
      expect(features['winning_cpm']).toBe(4.0);
      expect(features['latency_ms']).toBe(50);
      expect(features['cpm_ratio']).toBe(2.0);
    });

    it('should detect CTIT anomalies', () => {
      const input: ShadowScoreInput = {
        requestId: 'req-123',
        placementId: 'placement-abc',
        clickTimeToInstall: 5, // Very fast - suspicious
      };

      const features = extractFeatures(input);

      expect(features['ctit_seconds']).toBe(5);
      expect(features['ctit_anomaly']).toBe(1);
    });

    it('should not flag normal CTIT', () => {
      const input: ShadowScoreInput = {
        requestId: 'req-123',
        placementId: 'placement-abc',
        clickTimeToInstall: 60, // Normal
      };

      const features = extractFeatures(input);

      expect(features['ctit_seconds']).toBe(60);
      expect(features['ctit_anomaly']).toBe(0);
    });
  });

  describe('heuristicScore', () => {
    it('should score CTIT anomalies high', () => {
      const features = { ctit_anomaly: 1 };
      const result = heuristicScore(features);

      expect(result.score).toBeGreaterThanOrEqual(0.4);
      expect(result.reasons).toContain('extremely_fast_ctit');
    });

    it('should score missing device fingerprint', () => {
      const features = { has_ip: 0, has_user_agent: 0 };
      const result = heuristicScore(features);

      expect(result.score).toBeGreaterThanOrEqual(0.2);
      expect(result.reasons).toContain('missing_device_fingerprint');
    });

    it('should detect suspicious single high bid', () => {
      const features = { bid_count: 1, winning_cpm: 25 };
      const result = heuristicScore(features);

      expect(result.score).toBeGreaterThanOrEqual(0.15);
      expect(result.reasons).toContain('suspicious_single_bid_high_cpm');
    });

    it('should cap score at 1.0', () => {
      const features = {
        ctit_anomaly: 1,
        has_ip: 0,
        has_user_agent: 0,
        bid_count: 1,
        winning_cpm: 25,
        latency_ms: 1,
      };
      const result = heuristicScore(features);

      expect(result.score).toBeLessThanOrEqual(1.0);
    });

    it('should return low score for normal traffic', () => {
      const features = {
        has_ip: 1,
        has_user_agent: 1,
        bid_count: 5,
        winning_cpm: 2,
        latency_ms: 50,
      };
      const result = heuristicScore(features);

      expect(result.score).toBeLessThan(0.3);
    });
  });

  describe('scoreShadow', () => {
    it('should return a score result when ML is available', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: async () => ({ score: 0.25, reasons: ['ml_model_1'] }),
      });

      const input: ShadowScoreInput = {
        requestId: 'req-test-ml',
        placementId: 'placement-1',
        deviceInfo: { ip: '1.2.3.4' },
        auctionContext: { bidCount: 3 },
      };

      const result = await scoreShadow(input);

      expect(result).not.toBeNull();
      expect(result!.requestId).toBe('req-test-ml');
      expect(result!.score).toBe(0.25);
      expect(result!.riskBucket).toBe('low'); // 0.25 < 0.3
      expect(result!.modelVersion).toBe('stable');
    });

    it('should fallback to heuristics when ML fails', async () => {
      mockFetch.mockRejectedValueOnce(new Error('ML service down'));

      const input: ShadowScoreInput = {
        requestId: 'req-fallback',
        placementId: 'placement-1',
        clickTimeToInstall: 3, // CTIT anomaly
      };

      const result = await scoreShadow(input);

      expect(result).not.toBeNull();
      expect(result!.reasons).toContain('ml_fallback');
      expect(result!.score).toBeGreaterThan(0);
    });

    it('should classify risk buckets correctly', async () => {
      const testCases = [
        { score: 0.1, expected: 'low' },
        { score: 0.4, expected: 'medium' },
        { score: 0.7, expected: 'high' },
        { score: 0.9, expected: 'critical' },
      ];

      for (const tc of testCases) {
        mockFetch.mockResolvedValueOnce({
          ok: true,
          json: async () => ({ score: tc.score, reasons: [] }),
        });

        const result = await scoreShadow({
          requestId: `req-${tc.expected}`,
          placementId: 'placement-1',
        });

        expect(result!.riskBucket).toBe(tc.expected);
      }
    });

    it('should NEVER throw - always returns gracefully', async () => {
      mockFetch.mockRejectedValueOnce(new Error('Network error'));

      // Even with ML failure and heuristics, should not throw
      const result = await scoreShadow({
        requestId: 'req-resilient',
        placementId: 'placement-1',
      });

      // Should return result (fallback) or null, never throw
      expect(result === null || typeof result.score === 'number').toBe(true);
    });
  });

  describe('calculatePSI', () => {
    const { query } = require('../../../utils/postgres');

    it('should calculate PSI for production distribution', async () => {
      // Mock production distribution matching training
      query.mockResolvedValueOnce({
        rows: [
          { risk_bucket: 'low', proportion: '0.7' },
          { risk_bucket: 'medium', proportion: '0.2' },
          { risk_bucket: 'high', proportion: '0.08' },
          { risk_bucket: 'critical', proportion: '0.02' },
        ],
      });

      const result = await calculatePSI('stable', 24);

      expect(result.psi).toBeCloseTo(0, 1); // Should be ~0 when matching
      expect(result.driftDetected).toBe(false);
      expect(result.bucketComparison).toHaveLength(4);
    });

    it('should detect drift when distribution shifts', async () => {
      // Mock shifted distribution - much higher critical rate
      query.mockResolvedValueOnce({
        rows: [
          { risk_bucket: 'low', proportion: '0.3' },
          { risk_bucket: 'medium', proportion: '0.2' },
          { risk_bucket: 'high', proportion: '0.2' },
          { risk_bucket: 'critical', proportion: '0.3' },
        ],
      });

      const result = await calculatePSI('canary', 24);

      expect(result.psi).toBeGreaterThan(0.25);
      expect(result.driftDetected).toBe(true);
    });

    it('should handle empty results gracefully', async () => {
      query.mockResolvedValueOnce({ rows: [] });

      const result = await calculatePSI('new-model', 24);

      // Should not crash, return valid structure
      expect(result.bucketComparison).toBeDefined();
    });
  });
});
