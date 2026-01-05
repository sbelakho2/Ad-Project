import request from 'supertest';
import { Application } from 'express';

jest.mock('../../services/reportingService', () => ({
  __esModule: true,
  default: {
    getRevenueStats: jest.fn(),
    getTimeSeriesData: jest.fn(),
  },
}));

import reportingService from '../../services/reportingService';
import { createTestApp } from '../../__tests__/helpers/testApp';

describe('Revenue Controller', () => {
  let app: Application;
  const mockedReporting = jest.mocked(reportingService);

  beforeAll(() => {
    process.env.NODE_ENV = 'test';
    app = createTestApp();
  });

  beforeEach(() => {
    mockedReporting.getRevenueStats.mockResolvedValue({
      totalRevenue: 1000,
      totalImpressions: 5000,
      totalClicks: 50,
      ecpm: 20,
      ctr: 1,
      fillRate: 0,
    });
    mockedReporting.getTimeSeriesData.mockResolvedValue([
      { timestamp: '2025-01-01T00:00:00Z', revenue: 10, impressions: 100, clicks: 1, ecpm: 10 },
    ]);
  });

  test('prefers Idempotency-Key header over body/query', async () => {
    const res = await request(app)
      .get('/api/v1/revenue/summary?idempotencyKey=fromQuery')
      .set('Idempotency-Key', 'fromHeader')
      .send({ idempotencyKey: 'fromBody' });

    expect(res.status).toBe(200);
    // Header should be echoed back with the header value
    expect(res.headers['idempotency-key']).toBe('fromHeader');
    // Response meta should include the same
    expect(res.body?.meta?.idempotencyKey).toBe('fromHeader');
  });

  test('timeseries rejects invalid granularity', async () => {
    const res = await request(app)
      .get('/api/v1/revenue/timeseries?granularity=month');
    expect(res.status).toBe(400);
    expect(res.body).toMatchObject({ success: false });
  });

  test('summary rejects invalid date ranges', async () => {
    // summary uses default window via resolveDateRange but accepts query dates too through controller guard
    const res = await request(app)
      .get('/api/v1/revenue/summary?startDate=2025-12-31&endDate=2025-01-01');
    expect(res.status).toBe(400);
    expect(res.body?.error || res.body?.message).toBeTruthy();
  });

  test('date-only endDate is treated as inclusive day (exclusive boundary shifted)', async () => {
    const res = await request(app)
      .get('/api/v1/revenue/summary?startDate=2026-01-05&endDate=2026-01-05');

    expect(res.status).toBe(200);
    expect(mockedReporting.getRevenueStats).toHaveBeenCalledTimes(1);

    const [_publisherId, start, end] = mockedReporting.getRevenueStats.mock.calls[0];
    expect(start).toBeInstanceOf(Date);
    expect(end).toBeInstanceOf(Date);
    expect((start as Date).toISOString()).toBe('2026-01-05T00:00:00.000Z');
    expect((end as Date).toISOString()).toBe('2026-01-06T00:00:00.000Z');
  });
});
