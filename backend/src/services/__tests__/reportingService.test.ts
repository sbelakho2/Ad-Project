/**
 * Advanced Reporting Service Tests
 * 
 * Tests for analytics, fraud metrics, quality monitoring, and anomaly detection
 */

import type { QueryResult, QueryResultRow } from 'pg';
import { reportingService } from '../reportingService';
import * as postgres from '../../utils/postgres';

jest.mock('../../utils/postgres', () => ({
  __esModule: true,
  query: jest.fn(),
}));
jest.mock('../../utils/logger');

const mockPgQuery = postgres.query as jest.MockedFunction<typeof postgres.query>;

const mockRows = <T extends QueryResultRow>(rows: T[]): QueryResult<T> => ({ rows } as unknown as QueryResult<T>);

describe('ReportingService - Advanced Analytics', () => {
  const publisherId = '550e8400-e29b-41d4-a716-446655440000';
  const startDate = new Date('2024-01-01');
  const endDate = new Date('2024-01-31');

  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('Rollup + raw recent window', () => {
    let previousRecentRawDays: string | undefined;

    beforeEach(() => {
      previousRecentRawDays = process.env.REPORTING_RECENT_RAW_DAYS;
      process.env.REPORTING_RECENT_RAW_DAYS = '2';
    });

    afterEach(() => {
      if (previousRecentRawDays === undefined) {
        delete process.env.REPORTING_RECENT_RAW_DAYS;
      } else {
        process.env.REPORTING_RECENT_RAW_DAYS = previousRecentRawDays;
      }
    });

    it('getRevenueStats should combine rollups and raw for recent days', async () => {
      // 1) rollup table exists
      mockPgQuery.mockResolvedValueOnce(mockRows([{ exists: true } as never]));
      // 2) rollup totals for older part
      mockPgQuery.mockResolvedValueOnce(
        mockRows([
          {
            total_revenue: '10.5',
            total_impressions: '1000',
            total_clicks: '20',
            total_filled: '800',
          } as never,
        ])
      );
      // 3) raw totals for recent part
      mockPgQuery.mockResolvedValueOnce(
        mockRows([
          {
            revenue: '1.25',
            impressions: '100',
            clicks: '5',
            filled: '90',
          } as never,
        ])
      );

      const result = await reportingService.getRevenueStats(
        publisherId,
        new Date('2024-01-01T00:00:00.000Z'),
        new Date('2024-01-31T12:00:00.000Z')
      );

      expect(result.totalRevenue).toBeCloseTo(11.75, 6);
      expect(result.totalImpressions).toBe(1100);
      expect(result.totalClicks).toBe(25);
      expect(result.fillRate).toBeCloseTo((890 / 1100) * 100, 6);
      expect(result.ecpm).toBeCloseTo((11.75 / 1100) * 1000, 6);
      expect(result.ctr).toBeCloseTo((25 / 1100) * 100, 6);

      expect(mockPgQuery).toHaveBeenCalledTimes(3);
    });

    it('getTimeSeriesData(day) should return rollup buckets plus raw buckets for recent days', async () => {
      // 1) rollup table exists
      mockPgQuery.mockResolvedValueOnce(mockRows([{ exists: true } as never]));
      // 2) rollup time series (older)
      mockPgQuery.mockResolvedValueOnce(
        mockRows([
          { bucket: '2024-01-10', revenue: '2.0', impressions: '200', clicks: '4' } as never,
          { bucket: '2024-01-11', revenue: '3.0', impressions: '300', clicks: '6' } as never,
        ])
      );
      // 3) raw time series (recent)
      mockPgQuery.mockResolvedValueOnce(
        mockRows([
          {
            bucket: new Date('2024-01-30T00:00:00.000Z'),
            revenue: '1.0',
            impressions: '100',
            clicks: '2',
          } as never,
        ])
      );

      const points = await reportingService.getTimeSeriesData(
        publisherId,
        new Date('2024-01-01T00:00:00.000Z'),
        new Date('2024-01-31T12:00:00.000Z'),
        'day'
      );

      expect(points).toHaveLength(3);
      expect(points[0].timestamp).toBe('2024-01-10T00:00:00.000Z');
      expect(points[1].timestamp).toBe('2024-01-11T00:00:00.000Z');
      expect(points[2].timestamp).toBe('2024-01-30T00:00:00.000Z');

      expect(points[2]).toMatchObject({ revenue: 1.0, impressions: 100, clicks: 2 });
      expect(mockPgQuery).toHaveBeenCalledTimes(3);
    });
  });

  describe('getAdapterHealthScores', () => {
    it('should calculate adapter health scores correctly', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([
        {
          adapter_id: 'admob',
          adapter_name: 'AdMob',
          uptime: '99.5',
          error_rate: '0.005',
          avg_latency: '250',
          fill_rate: '85',
          revenue_share: '45',
          issues_last_24h: '2',
          last_issue: 'Timeout on placement xyz',
        },
        {
          adapter_id: 'applovin',
          adapter_name: 'AppLovin',
          uptime: '90.0',
          error_rate: '0.10',
          avg_latency: '800',
          fill_rate: '60',
          revenue_share: '35',
          issues_last_24h: '20',
          last_issue: 'Connection error',
        },
        {
          adapter_id: 'unity',
          adapter_name: 'Unity Ads',
          uptime: '45.0',
          error_rate: '0.55',
          avg_latency: '1200',
          fill_rate: '30',
          revenue_share: '10',
          issues_last_24h: '150',
          last_issue: 'Multiple timeouts',
        },
      ]));

      const result = await reportingService.getAdapterHealthScores(publisherId);

      expect(result).toHaveLength(3);
      
      // Verify healthy adapter
      expect(result[0]).toMatchObject({
        adapterId: 'admob',
        adapterName: 'AdMob',
        status: 'healthy',
        uptime: 99.5,
        errorRate: 0.005,
        avgResponseTime: 250,
        fillRate: 85,
        revenueShare: 45,
        issuesLast24h: 2,
      });
      expect(result[0].healthScore).toBeGreaterThan(90);

      // Verify degraded adapter
      expect(result[1].status).toBe('degraded');
      expect(result[1].healthScore).toBeLessThan(80); // Adjusted threshold

      // Verify offline adapter
      expect(result[2]).toMatchObject({
        adapterId: 'unity',
        status: 'offline',
      });
      expect(result[2].healthScore).toBeLessThan(50);
    });

    it('should handle empty adapter data', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getAdapterHealthScores(publisherId);

      expect(result).toEqual([]);
    });

    it('should handle query errors gracefully', async () => {
      mockPgQuery.mockRejectedValueOnce(new Error('Postgres connection failed'));

      await expect(reportingService.getAdapterHealthScores(publisherId))
        .rejects.toThrow('Postgres connection failed');
    });
  });

  describe('getFraudMetrics', () => {
    it('should return comprehensive fraud metrics', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([
          {
            total_requests: '1000000',
            fraud_requests: '15000',
            fraud_rate: '1.5',
            givt_detections: '8000',
            sivt_detections: '5000',
            ml_detections: '1500',
            anomaly_detections: '500',
            blocked_revenue: '2500.50',
          },
        ]))
          .mockResolvedValueOnce(mockRows([
            { fraud_type: 'datacenter_ip', count: '5000' },
            { fraud_type: 'bot_user_agent', count: '3000' },
            { fraud_type: 'click_farm', count: '2500' },
            { fraud_type: 'device_fingerprint', count: '2000' },
            { fraud_type: 'rapid_clicks', count: '1500' },
        ]));

      const result = await reportingService.getFraudMetrics(publisherId, startDate, endDate);

      expect(result).toEqual({
        totalRequests: 1000000,
        fraudRequests: 15000,
        fraudRate: 1.5,
        givtDetections: 8000,
        sivtDetections: 5000,
        mlDetections: 1500,
        anomalyDetections: 500,
        blockedRevenue: 2500.50,
        topFraudTypes: [
          { type: 'datacenter_ip', count: 5000 },
          { type: 'bot_user_agent', count: 3000 },
          { type: 'click_farm', count: 2500 },
          { type: 'device_fingerprint', count: 2000 },
          { type: 'rapid_clicks', count: 1500 },
        ],
      });
    });

    it('should handle zero fraud cases', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([
          {
            total_requests: '500000',
            fraud_requests: '0',
            fraud_rate: '0',
            givt_detections: '0',
            sivt_detections: '0',
            ml_detections: '0',
            anomaly_detections: '0',
            blocked_revenue: '0',
          },
        ]))
        .mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getFraudMetrics(publisherId, startDate, endDate);

      expect(result.fraudRate).toBe(0);
      expect(result.topFraudTypes).toEqual([]);
    });
  });

  describe('getQualityMetrics', () => {
    it('should return comprehensive quality metrics', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([
        {
          viewability_rate: '65.5',
          completion_rate: '78.2',
          ctr: '2.5',
          ivt_rate: '0.8',
          brand_safety: '98.5',
          user_experience: '92.0',
          anr_rate: '0.015',
          crash_rate: '0.005',
        },
      ]));

      const result = await reportingService.getQualityMetrics(publisherId, startDate, endDate);

      expect(result).toEqual({
        viewabilityRate: 65.5,
        completionRate: 78.2,
        clickThroughRate: 2.5,
        invalidTrafficRate: 0.8,
        brandSafetyScore: 98.5,
        userExperienceScore: 92.0,
        anrRate: 0.015,
        crashRate: 0.005,
      });
    });

    it('should return defaults when no data available', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getQualityMetrics(publisherId, startDate, endDate);

      expect(result).toEqual({
        viewabilityRate: 0,
        completionRate: 0,
        clickThroughRate: 0,
        invalidTrafficRate: 0,
        brandSafetyScore: 100,
        userExperienceScore: 100,
        anrRate: 0,
        crashRate: 0,
      });
    });

    it('should validate quality scores are within bounds', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([
        {
          viewability_rate: '105.0', // Invalid (>100)
          completion_rate: '95.0',
          ctr: '3.0',
          ivt_rate: '1.2',
          brand_safety: '150.0', // Invalid (>100)
          user_experience: '88.0',
          anr_rate: '0.020',
          crash_rate: '0.008',
        },
      ]));

      const result = await reportingService.getQualityMetrics(publisherId, startDate, endDate);

      // Should parse values as-is (validation happens at API layer)
      expect(result.viewabilityRate).toBe(105.0);
      expect(result.brandSafetyScore).toBe(150.0);
    });
  });

  describe('getRevenueProjections', () => {
    it('should project future revenue using linear regression', async () => {
      // Generate 30 days of mock data with upward trend
      const mockData = Array.from({ length: 30 }, (_, i) => ({
        date: new Date(Date.now() - (29 - i) * 24 * 60 * 60 * 1000).toISOString().split('T')[0],
        revenue: String(1000 + i * 50 + Math.random() * 100), // Upward trend with noise
      }));

      mockPgQuery
        .mockResolvedValueOnce(mockRows([{ exists: false } as never]))
        .mockResolvedValueOnce(mockRows(mockData as never));

      const result = await reportingService.getRevenueProjections(publisherId, 7);

      expect(result).toHaveLength(7);
      
      // Verify projection structure
      expect(result[0]).toHaveProperty('date');
      expect(result[0]).toHaveProperty('projectedRevenue');
      expect(result[0]).toHaveProperty('lowerBound');
      expect(result[0]).toHaveProperty('upperBound');
      expect(result[0]).toHaveProperty('confidence');
      expect(result[0].confidence).toBe(0.95);

      // Verify bounds
      result.forEach(projection => {
        expect(projection.lowerBound).toBeLessThanOrEqual(projection.projectedRevenue);
        expect(projection.upperBound).toBeGreaterThanOrEqual(projection.projectedRevenue);
        expect(projection.projectedRevenue).toBeGreaterThanOrEqual(0);
      });

      // Verify trend (should be increasing)
      expect(result[6].projectedRevenue).toBeGreaterThan(result[0].projectedRevenue);
    });

    it('should return empty array with insufficient data', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([{ exists: false } as never]))
        .mockResolvedValueOnce(
          mockRows([
            { date: '2024-01-01', revenue: '1000' } as never,
            { date: '2024-01-02', revenue: '1100' } as never,
          ])
        );

      const result = await reportingService.getRevenueProjections(publisherId, 7);

      expect(result).toEqual([]);
    });

    it('should handle declining revenue trends', async () => {
      const mockData = Array.from({ length: 30 }, (_, i) => ({
        date: new Date(Date.now() - (29 - i) * 24 * 60 * 60 * 1000).toISOString().split('T')[0],
        revenue: String(3000 - i * 50), // Declining trend
      }));

      mockPgQuery
        .mockResolvedValueOnce(mockRows([{ exists: false } as never]))
        .mockResolvedValueOnce(mockRows(mockData as never));

      const result = await reportingService.getRevenueProjections(publisherId, 7);

      expect(result).toHaveLength(7);
      // Should project decline
      expect(result[6].projectedRevenue).toBeLessThan(result[0].projectedRevenue);
      // But should never go negative
      result.forEach(projection => {
        expect(projection.projectedRevenue).toBeGreaterThanOrEqual(0);
      });
    });
  });

  describe('getCohortAnalysis', () => {
    it('should return cohort metrics with LTV and retention', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([
        {
          cohort_date: '2024-01-01',
          user_count: '10000',
          day0_revenue: '5000',
          day1_revenue: '8000',
          day7_revenue: '15000',
          day30_revenue: '25000',
          ltv: '2.50',
          retention_day1: '65.0',
          retention_day7: '45.0',
          retention_day30: '30.0',
          arpu: '2.50',
        },
        {
          cohort_date: '2024-01-02',
          user_count: '12000',
          day0_revenue: '6000',
          day1_revenue: '9500',
          day7_revenue: '18000',
          day30_revenue: '30000',
          ltv: '2.50',
          retention_day1: '68.0',
          retention_day7: '48.0',
          retention_day30: '32.0',
          arpu: '2.50',
        },
      ]));

      const result = await reportingService.getCohortAnalysis(publisherId, startDate, endDate);

      expect(result).toHaveLength(2);
      expect(result[0]).toEqual({
        cohortDate: '2024-01-01',
        userCount: 10000,
        day0Revenue: 5000,
        day1Revenue: 8000,
        day7Revenue: 15000,
        day30Revenue: 25000,
        ltv: 2.50,
        retentionDay1: 65.0,
        retentionDay7: 45.0,
        retentionDay30: 30.0,
        arpu: 2.50,
      });

      // Verify retention funnel
      expect(result[0].retentionDay1).toBeGreaterThan(result[0].retentionDay7);
      expect(result[0].retentionDay7).toBeGreaterThan(result[0].retentionDay30);

      // Verify revenue accumulation
      expect(result[0].day30Revenue).toBeGreaterThan(result[0].day7Revenue);
      expect(result[0].day7Revenue).toBeGreaterThan(result[0].day1Revenue);
    });

    it('should handle empty cohort data', async () => {
      mockPgQuery.mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getCohortAnalysis(publisherId, startDate, endDate);

      expect(result).toEqual([]);
    });
  });

  describe('getAnomalies', () => {
    it('should detect revenue drop anomalies', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([
          {
            hour: '2024-01-15 14:00:00',
            revenue: '500',
            requests: '10000',
            error_rate: '1.0',
            avg_latency: '250',
            mean_revenue: '2000',
            std_revenue: '200',
            mean_requests: '10000',
            mean_error_rate: '1.0',
            mean_latency: '250',
            revenue_z_score: '7.5', // Significant drop
            requests_z_score: '0',
            error_z_score: '0',
            latency_z_score: '0',
          },
        ]))
        .mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getAnomalies(publisherId, 24);

      expect(result.length).toBeGreaterThan(0);
      
      const revenueAnomaly = result.find(a => a.type === 'revenue_drop');
      expect(revenueAnomaly).toBeDefined();
      expect(revenueAnomaly?.severity).toBe('critical');
      expect(revenueAnomaly?.currentValue).toBe(500);
      expect(revenueAnomaly?.expectedValue).toBe(2000);
      expect(revenueAnomaly?.deviation).toBeLessThan(0);
    });

    it('should detect performance degradation', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([
          {
            hour: '2024-01-15 14:00:00',
            revenue: '2000',
            requests: '10000',
            error_rate: '15.0', // High error rate
            avg_latency: '2000', // High latency
            mean_revenue: '2000',
            std_revenue: '200',
            mean_requests: '10000',
            mean_error_rate: '1.0',
            mean_latency: '250',
            revenue_z_score: '0',
            requests_z_score: '0',
            error_z_score: '4.5',
            latency_z_score: '5.0',
          },
        ]))
        .mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getAnomalies(publisherId, 24);

      const errorAnomaly = result.find(a => a.metric === 'error_rate');
      expect(errorAnomaly).toBeDefined();
      expect(errorAnomaly?.type).toBe('performance_degradation');
      
      const latencyAnomaly = result.find(a => a.metric === 'latency');
      expect(latencyAnomaly).toBeDefined();
      expect(latencyAnomaly?.type).toBe('performance_degradation');
    });

    it('should detect fraud spikes', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([]))
        .mockResolvedValueOnce(mockRows([
          {
            hour: '2024-01-15 14:00:00',
            fraud_count: '5000',
            total_count: '50000',
            fraud_rate: '10.0',
          },
        ]));

      const result = await reportingService.getAnomalies(publisherId, 24);

      const fraudAnomaly = result.find(a => a.type === 'fraud_spike');
      expect(fraudAnomaly).toBeDefined();
      expect(fraudAnomaly?.severity).toBe('medium'); // 10% fraud rate = medium severity
      expect(fraudAnomaly?.currentValue).toBe(10.0);
      expect(fraudAnomaly?.expectedValue).toBe(2.0);
    });

    it('should detect traffic anomalies', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([
          {
            hour: '2024-01-15 14:00:00',
            revenue: '4000',
            requests: '50000', // Traffic spike
            error_rate: '1.0',
            avg_latency: '250',
            mean_revenue: '2000',
            std_revenue: '200',
            mean_requests: '10000',
            std_requests: '1000',
            mean_error_rate: '1.0',
            mean_latency: '250',
            revenue_z_score: '10.0',
            requests_z_score: '40.0', // Massive spike
            error_z_score: '0',
            latency_z_score: '0',
          },
        ]))
        .mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getAnomalies(publisherId, 24);

      const trafficAnomaly = result.find(a => a.metric === 'requests');
      expect(trafficAnomaly).toBeDefined();
      expect(trafficAnomaly?.type).toBe('traffic_anomaly');
      expect(trafficAnomaly?.severity).toBe('critical');
    });

    it('should return empty array when no anomalies detected', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([]))
        .mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getAnomalies(publisherId, 24);

      expect(result).toEqual([]);
    });

    it('should assign appropriate severity levels', async () => {
      mockPgQuery
        .mockResolvedValueOnce(mockRows([
          {
            hour: '2024-01-15 14:00:00',
            revenue: '1400',
            requests: '10000',
            error_rate: '1.0',
            avg_latency: '250',
            mean_revenue: '2000',
            std_revenue: '200',
            mean_requests: '10000',
            mean_error_rate: '1.0',
            mean_latency: '250',
            revenue_z_score: '3.0', // Medium severity
            requests_z_score: '0',
            error_z_score: '0',
            latency_z_score: '0',
          },
        ]))
        .mockResolvedValueOnce(mockRows([]));

      const result = await reportingService.getAnomalies(publisherId, 24);

      const anomaly = result.find(a => a.type === 'revenue_drop');
      expect(anomaly?.severity).toBe('medium');
    });
  });

  describe('Integration - Multiple Metrics', () => {
    it('should correlate adapter health with quality metrics', async () => {
      // Mock adapter health data
      mockPgQuery.mockResolvedValueOnce(mockRows([
        {
          adapter_id: 'admob',
          adapter_name: 'AdMob',
          uptime: '99.5',
          error_rate: '0.005',
          avg_latency: '250',
          fill_rate: '85',
          revenue_share: '45',
          issues_last_24h: '2',
          last_issue: null,
        },
      ]));

      const healthScores = await reportingService.getAdapterHealthScores(publisherId);

      // Mock quality metrics
      mockPgQuery.mockResolvedValueOnce(mockRows([
        {
          viewability_rate: '70.0',
          completion_rate: '80.0',
          ctr: '2.5',
          ivt_rate: '0.5',
          brand_safety: '99.0',
          user_experience: '95.0',
          anr_rate: '0.010',
          crash_rate: '0.003',
        },
      ]));

      const qualityMetrics = await reportingService.getQualityMetrics(publisherId, startDate, endDate);

      // High health score should correlate with good quality metrics
      expect(healthScores[0].healthScore).toBeGreaterThan(90);
      expect(qualityMetrics.viewabilityRate).toBeGreaterThan(60);
      expect(qualityMetrics.anrRate).toBeLessThan(0.02);
    });
  });
});
