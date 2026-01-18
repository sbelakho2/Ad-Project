import { describe, it, expect } from '@jest/globals';
import { buildAnalyticsTimeSeriesQuery } from '../../../src/services/analyticsQueryService';

describe('AnalyticsQueryService', () => {
  it('builds time-series queries with safe defaults', () => {
    const startDate = new Date('2026-01-01T00:00:00Z');
    const endDate = new Date('2026-01-02T00:00:00Z');

    const result = buildAnalyticsTimeSeriesQuery({
      table: 'analytics_impressions',
      metric: 'count',
      startDate,
      endDate,
    });

    expect(result.sql).toContain('FROM analytics_impressions');
    expect(result.sql).toContain('date_trunc($1, observed_at)');
    expect(result.sql).toContain('is_test_mode = false');
    expect(result.params).toEqual(['day', startDate, endDate]);
  });

  it('rejects revenue queries for non-revenue tables', () => {
    const startDate = new Date('2026-01-01T00:00:00Z');
    const endDate = new Date('2026-01-02T00:00:00Z');

    expect(() =>
      buildAnalyticsTimeSeriesQuery({
        table: 'analytics_clicks',
        metric: 'revenue',
        startDate,
        endDate,
      })
    ).toThrow('Revenue metric is only supported for analytics_revenue_events');
  });
});
