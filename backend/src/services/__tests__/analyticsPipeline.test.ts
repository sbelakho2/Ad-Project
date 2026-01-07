import { subDays } from 'date-fns';

// Mock reportingService (the actual dependency of analyticsPipeline)
const mockGetRevenueStats = jest.fn();
const mockGetTimeSeriesData = jest.fn();
const mockGetAdapterPerformance = jest.fn();

jest.mock('../reportingService', () => ({
  __esModule: true,
  default: {
    getRevenueStats: (...args: unknown[]) => mockGetRevenueStats(...args),
    getTimeSeriesData: (...args: unknown[]) => mockGetTimeSeriesData(...args),
    getAdapterPerformance: (...args: unknown[]) => mockGetAdapterPerformance(...args),
  },
}));

// Mock postgres query for placement breakdown
const mockQuery = jest.fn();
jest.mock('../../utils/postgres', () => ({
  query: (...args: unknown[]) => mockQuery(...args),
}));

import {
  getAnalyticsOverview,
  getAnalyticsTimeSeries,
  getPerformanceBreakdown,
} from '../analyticsPipeline';

const publisherId = 'publisher-1';

describe('analyticsPipeline service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.useRealTimers();
  });

  it('delegates overview fetch to reportingService', async () => {
    // Mock all the revenue stats calls made by getAnalyticsOverview
    mockGetRevenueStats.mockResolvedValue({
      totalRevenue: 100,
      totalImpressions: 1000,
      totalClicks: 50,
      ecpm: 5,
      ctr: 5,
    });

    const result = await getAnalyticsOverview(publisherId);

    // Should call getRevenueStats 5 times for different date ranges
    expect(mockGetRevenueStats).toHaveBeenCalledTimes(5);
    expect(result.revenue.lifetime).toBe(100);
    expect(result.impressions).toBe(1000);
    expect(result.clicks).toBe(50);
  });

  it('returns formatted time series with default 30-day range when no dates provided', async () => {
    const now = new Date('2024-04-30T12:00:00.000Z');
    jest.useFakeTimers().setSystemTime(now);

    mockGetTimeSeriesData.mockResolvedValue([
      {
        timestamp: '2024-04-29',
        revenue: 12.345,
        impressions: 1000,
        clicks: 120,
      },
    ]);

    const result = await getAnalyticsTimeSeries(publisherId, 'revenue');

    expect(mockGetTimeSeriesData).toHaveBeenCalledWith(
      publisherId,
      expect.any(Date),
      expect.any(Date),
      'day'
    );
    expect(result).toEqual([
      {
        date: '2024-04-29',
        value: 12.35,
        revenue: 12.345,
        impressions: 1000,
        clicks: 120,
      },
    ]);
  });

  it('calculates metric-specific values and uses provided range', async () => {
    mockGetTimeSeriesData.mockResolvedValue([
      {
        timestamp: '2024-04-28',
        revenue: 5.01,
        impressions: 200,
        clicks: 10,
      },
    ]);

    const start = '2024-04-01';
    const end = '2024-04-30';

    const impressionsSeries = await getAnalyticsTimeSeries(
      publisherId,
      'impressions',
      start,
      end
    );

    expect(mockGetTimeSeriesData).toHaveBeenCalledWith(
      publisherId,
      new Date(start),
      new Date(end),
      'day'
    );
    expect(impressionsSeries[0]).toMatchObject({
      value: 200,
      impressions: 200,
      clicks: 10,
      revenue: 5.01,
    });

    mockGetTimeSeriesData.mockResolvedValue([
      {
        timestamp: '2024-04-28',
        revenue: 5.01,
        impressions: 200,
        clicks: 10,
      },
    ]);
    const clicksSeries = await getAnalyticsTimeSeries(publisherId, 'clicks', start, end);
    expect(clicksSeries[0].value).toBe(10);
  });

  it('delegates adapter performance breakdown to reportingService', async () => {
    mockGetAdapterPerformance.mockResolvedValue([
      {
        adapterId: 'adapter-1',
        adapterName: 'AdMob',
        revenue: 10,
        impressions: 500,
        ecpm: 20,
      },
    ]);

    const result = await getPerformanceBreakdown(publisherId, 'adapter');

    expect(mockGetAdapterPerformance).toHaveBeenCalledWith(
      publisherId,
      expect.any(Date),
      expect.any(Date)
    );
    expect(result[0]).toMatchObject({
      id: 'adapter-1',
      name: 'AdMob',
      revenue: 10,
      impressions: 500,
      ecpm: 20,
    });
  });

  it('delegates placement performance breakdown to direct query', async () => {
    mockQuery.mockResolvedValue({
      rows: [
        {
          placement_id: 'placement-1',
          placement_name: 'Interstitial',
          revenue: '10',
          impressions: '500',
        },
      ],
    });

    const result = await getPerformanceBreakdown(publisherId, 'placement');

    expect(mockQuery).toHaveBeenCalledWith(
      expect.stringContaining('analytics_revenue_events'),
      expect.arrayContaining([publisherId]),
      expect.objectContaining({ replica: true })
    );
    expect(result[0]).toMatchObject({
      id: 'placement-1',
      name: 'Interstitial',
    });
  });
});
