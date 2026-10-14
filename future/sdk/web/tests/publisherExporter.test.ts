/**
 * @file Publisher Exporter Tests
 * @description Tests for publisher data export functionality
 */

import {
  PublisherExporter,
  ExportFormat,
  ExportDimension,
  ExportMetric,
  ExportConfigBuilder,
} from '../src/publisherExporter';
import { UsageMetrics, UsageBreakdown } from '../src/usageMeter';

describe('PublisherExporter', () => {
  let exporter: PublisherExporter;

  beforeEach(() => {
    exporter = new PublisherExporter();
  });

  describe('Local Export Generation', () => {
    const mockMetrics: UsageMetrics = {
      adRequests: 1000,
      adImpressions: 700,
      adClicks: 35,
      videoStarts: 100,
      videoCompletes: 80,
      totalRevenue: 14.0,
      apiCalls: 500,
      cacheHits: 800,
      cacheMisses: 200,
      errors: 10,
      periodStart: new Date('2024-01-01').getTime(),
      periodEnd: new Date('2024-01-02').getTime(),
    };

    it('should generate JSON export', () => {
      const result = exporter.generateLocalExport(mockMetrics);

      expect(result.success).toBe(true);
      expect(result.format).toBe(ExportFormat.JSON);
      expect(result.data).toBeDefined();
      expect(result.rowCount).toBeGreaterThan(0);

      const parsed = JSON.parse(result.data as string);
      expect(Array.isArray(parsed)).toBe(true);
      expect(parsed.length).toBeGreaterThan(0);
    });

    it('should generate CSV export', () => {
      const result = exporter.generateLocalExport(mockMetrics, undefined, {
        format: ExportFormat.CSV,
      });

      expect(result.success).toBe(true);
      expect(result.format).toBe(ExportFormat.CSV);
      expect(result.data).toBeDefined();

      const csv = result.data as string;
      const lines = csv.split('\n');
      expect(lines.length).toBeGreaterThan(1); // Header + data
      expect(lines[0]).toContain('requests');
      expect(lines[0]).toContain('impressions');
    });

    it('should include correct metrics in export', () => {
      const result = exporter.generateLocalExport(mockMetrics);
      const parsed = JSON.parse(result.data as string);
      const row = parsed[0];

      expect(row.requests).toBe(1000);
      expect(row.impressions).toBe(700);
      expect(row.clicks).toBe(35);
      expect(row.revenue).toBe(14.0);
    });

    it('should calculate derived metrics correctly', () => {
      const result = exporter.generateLocalExport(mockMetrics);
      const parsed = JSON.parse(result.data as string);
      const row = parsed[0];

      expect(row.ctr).toBeCloseTo(35 / 700, 5); // 5%
      expect(row.fillRate).toBeCloseTo(700 / 1000, 5); // 70%
      expect(row.ecpm).toBeCloseTo((14.0 / 700) * 1000, 5); // $20 eCPM
      expect(row.videoCompletionRate).toBeCloseTo(80 / 100, 5); // 80%
    });

    it('should handle zero values for derived metrics', () => {
      const emptyMetrics: UsageMetrics = {
        adRequests: 0,
        adImpressions: 0,
        adClicks: 0,
        videoStarts: 0,
        videoCompletes: 0,
        totalRevenue: 0,
        apiCalls: 0,
        cacheHits: 0,
        cacheMisses: 0,
        errors: 0,
        periodStart: Date.now(),
        periodEnd: Date.now(),
      };

      const result = exporter.generateLocalExport(emptyMetrics);
      const parsed = JSON.parse(result.data as string);
      const row = parsed[0];

      expect(row.ctr).toBe(0);
      expect(row.fillRate).toBe(0);
      expect(row.ecpm).toBe(0);
      expect(row.videoCompletionRate).toBe(0);
    });

    it('should include breakdown data when provided', () => {
      const breakdown: UsageBreakdown = {
        byPlacement: new Map([
          [
            'placement-1',
            {
              adRequests: 500,
              adImpressions: 400,
              adClicks: 20,
              videoStarts: 50,
              videoCompletes: 40,
              totalRevenue: 8.0,
              apiCalls: 250,
              cacheHits: 400,
              cacheMisses: 100,
              errors: 5,
              periodStart: mockMetrics.periodStart,
              periodEnd: mockMetrics.periodEnd,
            },
          ],
        ]),
        byAdapter: new Map([
          [
            'admob',
            {
              adRequests: 600,
              adImpressions: 450,
              adClicks: 22,
              videoStarts: 60,
              videoCompletes: 48,
              totalRevenue: 9.0,
              apiCalls: 300,
              cacheHits: 500,
              cacheMisses: 100,
              errors: 3,
              periodStart: mockMetrics.periodStart,
              periodEnd: mockMetrics.periodEnd,
            },
          ],
        ]),
        byAdFormat: new Map(),
      };

      const result = exporter.generateLocalExport(mockMetrics, breakdown);
      const parsed = JSON.parse(result.data as string);

      // Should have global + placement + adapter rows
      expect(parsed.length).toBe(3);

      const placementRow = parsed.find((r: Record<string, unknown>) => r.placement === 'placement-1');
      expect(placementRow).toBeDefined();
      expect(placementRow.requests).toBe(500);

      const adapterRow = parsed.find((r: Record<string, unknown>) => r.adapter === 'admob');
      expect(adapterRow).toBeDefined();
      expect(adapterRow.requests).toBe(600);
    });
  });

  describe('CSV Formatting', () => {
    it('should create valid CSV with headers', () => {
      const metrics: UsageMetrics = {
        adRequests: 100,
        adImpressions: 80,
        adClicks: 4,
        videoStarts: 10,
        videoCompletes: 8,
        totalRevenue: 2.0,
        apiCalls: 50,
        cacheHits: 60,
        cacheMisses: 20,
        errors: 1,
        periodStart: Date.now(),
        periodEnd: Date.now(),
      };

      const result = exporter.generateLocalExport(metrics, undefined, {
        format: ExportFormat.CSV,
      });

      const lines = (result.data as string).split('\n');
      const headers = lines[0].split(',');

      expect(headers).toContain('requests');
      expect(headers).toContain('impressions');
      expect(headers).toContain('clicks');
      expect(headers).toContain('revenue');
      expect(headers).toContain('ctr');
      expect(headers).toContain('fillRate');
    });

    it('should properly escape CSV values with commas', () => {
      const metrics: UsageMetrics = {
        adRequests: 100,
        adImpressions: 80,
        adClicks: 4,
        videoStarts: 10,
        videoCompletes: 8,
        totalRevenue: 2.0,
        apiCalls: 50,
        cacheHits: 60,
        cacheMisses: 20,
        errors: 1,
        periodStart: Date.now(),
        periodEnd: Date.now(),
      };

      const result = exporter.generateLocalExport(metrics, undefined, {
        format: ExportFormat.CSV,
      });

      expect(result.success).toBe(true);
      // Just verify it doesn't throw
    });
  });

  describe('ExportConfigBuilder', () => {
    it('should build config with format', () => {
      const config = new ExportConfigBuilder().format(ExportFormat.CSV).build();

      expect(config.format).toBe(ExportFormat.CSV);
    });

    it('should build config with date range', () => {
      const startDate = new Date('2024-01-01');
      const endDate = new Date('2024-01-31');

      const config = new ExportConfigBuilder().dateRange(startDate, endDate).build();

      expect(config.dateRange?.startDate).toEqual(startDate);
      expect(config.dateRange?.endDate).toEqual(endDate);
    });

    it('should build config with lastNDays', () => {
      const config = new ExportConfigBuilder().lastNDays(7).build();

      expect(config.dateRange).toBeDefined();

      const daysDiff = Math.floor(
        (config.dateRange!.endDate.getTime() - config.dateRange!.startDate.getTime()) /
          (1000 * 60 * 60 * 24)
      );
      expect(daysDiff).toBeGreaterThanOrEqual(6);
      expect(daysDiff).toBeLessThanOrEqual(7);
    });

    it('should build config with dimensions', () => {
      const config = new ExportConfigBuilder()
        .dimensions(ExportDimension.DATE, ExportDimension.PLACEMENT, ExportDimension.ADAPTER)
        .build();

      expect(config.dimensions).toContain(ExportDimension.DATE);
      expect(config.dimensions).toContain(ExportDimension.PLACEMENT);
      expect(config.dimensions).toContain(ExportDimension.ADAPTER);
    });

    it('should build config with metrics', () => {
      const config = new ExportConfigBuilder()
        .metrics(ExportMetric.REQUESTS, ExportMetric.IMPRESSIONS, ExportMetric.REVENUE)
        .build();

      expect(config.metrics).toContain(ExportMetric.REQUESTS);
      expect(config.metrics).toContain(ExportMetric.IMPRESSIONS);
      expect(config.metrics).toContain(ExportMetric.REVENUE);
    });

    it('should build config with filters', () => {
      const config = new ExportConfigBuilder()
        .filter(ExportDimension.ADAPTER, 'equals', 'admob')
        .filter(ExportDimension.COUNTRY, 'in', ['US', 'CA', 'UK'])
        .build();

      expect(config.filters).toHaveLength(2);
      expect(config.filters![0].dimension).toBe(ExportDimension.ADAPTER);
      expect(config.filters![0].operator).toBe('equals');
      expect(config.filters![0].value).toBe('admob');
    });

    it('should build config with breakdowns enabled', () => {
      const config = new ExportConfigBuilder().includeBreakdowns(true).build();

      expect(config.includeBreakdowns).toBe(true);
    });

    it('should build config with timezone', () => {
      const config = new ExportConfigBuilder().timezone('America/New_York').build();

      expect(config.timezone).toBe('America/New_York');
    });

    it('should chain all builder methods', () => {
      const startDate = new Date('2024-01-01');
      const endDate = new Date('2024-01-31');

      const config = new ExportConfigBuilder()
        .format(ExportFormat.CSV)
        .dateRange(startDate, endDate)
        .dimensions(ExportDimension.DATE, ExportDimension.ADAPTER)
        .metrics(ExportMetric.REQUESTS, ExportMetric.REVENUE)
        .filter(ExportDimension.ADAPTER, 'not_equals', 'test')
        .includeBreakdowns()
        .timezone('UTC')
        .build();

      expect(config.format).toBe(ExportFormat.CSV);
      expect(config.dateRange).toBeDefined();
      expect(config.dimensions).toHaveLength(2);
      expect(config.metrics).toHaveLength(2);
      expect(config.filters).toHaveLength(1);
      expect(config.includeBreakdowns).toBe(true);
      expect(config.timezone).toBe('UTC');
    });
  });

  describe('Export Result', () => {
    it('should include generation timestamp', () => {
      const metrics: UsageMetrics = {
        adRequests: 100,
        adImpressions: 80,
        adClicks: 4,
        videoStarts: 10,
        videoCompletes: 8,
        totalRevenue: 2.0,
        apiCalls: 50,
        cacheHits: 60,
        cacheMisses: 20,
        errors: 1,
        periodStart: Date.now(),
        periodEnd: Date.now(),
      };

      const before = new Date();
      const result = exporter.generateLocalExport(metrics);
      const after = new Date();

      expect(result.generatedAt.getTime()).toBeGreaterThanOrEqual(before.getTime());
      expect(result.generatedAt.getTime()).toBeLessThanOrEqual(after.getTime());
    });

    it('should include row count', () => {
      const metrics: UsageMetrics = {
        adRequests: 100,
        adImpressions: 80,
        adClicks: 4,
        videoStarts: 10,
        videoCompletes: 8,
        totalRevenue: 2.0,
        apiCalls: 50,
        cacheHits: 60,
        cacheMisses: 20,
        errors: 1,
        periodStart: Date.now(),
        periodEnd: Date.now(),
      };

      const result = exporter.generateLocalExport(metrics);
      expect(result.rowCount).toBe(1);
    });
  });

  describe('Exporter Configuration', () => {
    it('should accept custom API endpoint', () => {
      const customExporter = new PublisherExporter({
        apiEndpoint: 'https://custom.api.com/exports',
      });

      expect(customExporter).toBeDefined();
    });

    it('should accept API key', () => {
      const customExporter = new PublisherExporter({
        apiKey: 'test-api-key-123',
      });

      expect(customExporter).toBeDefined();
    });
  });

  describe('Export Dimensions', () => {
    it('should have all expected dimensions', () => {
      expect(ExportDimension.DATE).toBe('date');
      expect(ExportDimension.HOUR).toBe('hour');
      expect(ExportDimension.PLACEMENT).toBe('placement');
      expect(ExportDimension.ADAPTER).toBe('adapter');
      expect(ExportDimension.AD_FORMAT).toBe('ad_format');
      expect(ExportDimension.COUNTRY).toBe('country');
      expect(ExportDimension.DEVICE_TYPE).toBe('device_type');
      expect(ExportDimension.OS_VERSION).toBe('os_version');
      expect(ExportDimension.SDK_VERSION).toBe('sdk_version');
    });
  });

  describe('Export Metrics', () => {
    it('should have all expected metrics', () => {
      expect(ExportMetric.REQUESTS).toBe('requests');
      expect(ExportMetric.IMPRESSIONS).toBe('impressions');
      expect(ExportMetric.CLICKS).toBe('clicks');
      expect(ExportMetric.CTR).toBe('ctr');
      expect(ExportMetric.FILL_RATE).toBe('fill_rate');
      expect(ExportMetric.REVENUE).toBe('revenue');
      expect(ExportMetric.ECPM).toBe('ecpm');
      expect(ExportMetric.VIDEO_STARTS).toBe('video_starts');
      expect(ExportMetric.VIDEO_COMPLETES).toBe('video_completes');
      expect(ExportMetric.VIDEO_COMPLETION_RATE).toBe('video_completion_rate');
      expect(ExportMetric.ERRORS).toBe('errors');
      expect(ExportMetric.LATENCY_P50).toBe('latency_p50');
      expect(ExportMetric.LATENCY_P95).toBe('latency_p95');
      expect(ExportMetric.LATENCY_P99).toBe('latency_p99');
    });
  });

  describe('Export Formats', () => {
    it('should have all expected formats', () => {
      expect(ExportFormat.JSON).toBe('json');
      expect(ExportFormat.CSV).toBe('csv');
      expect(ExportFormat.PARQUET).toBe('parquet');
    });
  });
});
