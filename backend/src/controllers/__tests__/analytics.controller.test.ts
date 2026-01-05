import request from 'supertest';
import { Application } from 'express';

jest.mock('../../services/analyticsPipeline', () => ({
  __esModule: true,
  getAnalyticsOverview: jest.fn(),
  getAnalyticsTimeSeries: jest.fn(),
  getPerformanceBreakdown: jest.fn(),
  getImpressionsBreakdown: jest.fn(),
}));

import {
  getAnalyticsOverview,
  getAnalyticsTimeSeries,
  getPerformanceBreakdown,
  getImpressionsBreakdown,
} from '../../services/analyticsPipeline';

import { createTestApp } from '../../__tests__/helpers/testApp';

describe('Analytics Controller', () => {
  let app: Application;

  const mockedGetAnalyticsOverview = jest.mocked(getAnalyticsOverview);
  const mockedGetAnalyticsTimeSeries = jest.mocked(getAnalyticsTimeSeries);
  const mockedGetPerformanceBreakdown = jest.mocked(getPerformanceBreakdown);
  const mockedGetImpressionsBreakdown = jest.mocked(getImpressionsBreakdown);

  beforeAll(() => {
    process.env.NODE_ENV = 'test';
    app = createTestApp();
  });

  beforeEach(() => {
    mockedGetAnalyticsOverview.mockResolvedValue({
      revenue: { today: 1, yesterday: 2, thisMonth: 3, lastMonth: 4, lifetime: 5 },
      impressions: 100,
      clicks: 10,
      ecpm: 12.34,
      ctr: 1.23,
    } as any);

    mockedGetAnalyticsTimeSeries.mockResolvedValue([
      { date: '2026-01-01', value: 1, revenue: 1, impressions: 10, clicks: 0 },
    ] as any);

    mockedGetPerformanceBreakdown.mockResolvedValue([
      { id: 'x', name: 'X', revenue: 1, impressions: 10, ecpm: 100 },
    ] as any);

    mockedGetImpressionsBreakdown.mockResolvedValue([
      { groupBy: 'date', date: '2026-01-01', impressions: 10 },
    ] as any);
  });

  test('overview delegates to analyticsPipeline', async () => {
    const res = await request(app).get('/api/v1/analytics/overview');
    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ success: true });
    expect(mockedGetAnalyticsOverview).toHaveBeenCalledWith('pub-1');
  });

  test('timeseries passes metric/date range and granularity', async () => {
    const res = await request(app).get(
      '/api/v1/analytics/timeseries?metric=impressions&startDate=2026-01-01&endDate=2026-01-02&granularity=hour'
    );

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ success: true });

    expect(mockedGetAnalyticsTimeSeries).toHaveBeenCalledWith(
      'pub-1',
      'impressions',
      '2026-01-01',
      '2026-01-02',
      'hour'
    );
  });

  test('performance passes groupBy and optional date range', async () => {
    const res = await request(app).get(
      '/api/v1/analytics/performance?groupBy=adapter&startDate=2026-01-01&endDate=2026-01-02'
    );

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ success: true });

    expect(mockedGetPerformanceBreakdown).toHaveBeenCalledWith('pub-1', 'adapter', '2026-01-01', '2026-01-02');
  });

  test('impressions route exists and defaults groupBy=date', async () => {
    const res = await request(app).get('/api/v1/analytics/impressions?startDate=2026-01-01&endDate=2026-01-02');

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ success: true });

    expect(mockedGetImpressionsBreakdown).toHaveBeenCalledWith('pub-1', 'date', '2026-01-01', '2026-01-02');
  });
});
