import { AnalyticsService } from '../analyticsService';
import { insertMany, query } from '../../utils/postgres';
import {
  ImpressionEventDTO,
  ClickEventDTO,
  RevenueEventDTO,
} from '../../types/analytics.types';

jest.mock('../../utils/postgres', () => ({
  insertMany: jest.fn().mockResolvedValue(undefined),
  query: jest.fn().mockImplementation((sql: string, params?: unknown[]) => {
    if (typeof sql === 'string' && sql.includes('information_schema.columns')) {
      const tableName = Array.isArray(params) ? String(params[0] ?? '') : '';

      const columnsByTable: Record<string, string[]> = {
        analytics_impressions: [
          'id',
          'created_at',
          'event_id',
          'observed_at',
          'publisher_id',
          'app_id',
          'placement_id',
          'adapter_id',
          'adapter_name',
          'ad_unit_id',
          'ad_format',
          'country_code',
          'device_type',
          'os',
          'os_version',
          'session_id',
          'user_id',
          'request_id',
          'status',
          'filled',
          'latency_ms',
          'meta',
          'is_test_mode',
        ],
        analytics_impressions_stage: [
          'id',
          'created_at',
          'event_id',
          'observed_at',
          'publisher_id',
          'app_id',
          'placement_id',
          'adapter_id',
          'adapter_name',
          'ad_unit_id',
          'ad_format',
          'country_code',
          'device_type',
          'os',
          'os_version',
          'session_id',
          'user_id',
          'request_id',
          'status',
          'filled',
          'latency_ms',
          'meta',
          'is_test_mode',
        ],
        analytics_clicks: [
          'id',
          'created_at',
          'event_id',
          'observed_at',
          'impression_id',
          'publisher_id',
          'app_id',
          'placement_id',
          'adapter_id',
          'adapter_name',
          'click_url',
          'country_code',
          'device_type',
          'os',
          'session_id',
          'user_id',
          'request_id',
          'time_to_click_ms',
          'is_verified',
          'meta',
          'is_test_mode',
        ],
        analytics_clicks_stage: [
          'id',
          'created_at',
          'event_id',
          'observed_at',
          'impression_id',
          'publisher_id',
          'app_id',
          'placement_id',
          'adapter_id',
          'adapter_name',
          'click_url',
          'country_code',
          'device_type',
          'os',
          'session_id',
          'user_id',
          'request_id',
          'time_to_click_ms',
          'is_verified',
          'meta',
          'is_test_mode',
        ],
        analytics_revenue_events: [
          'id',
          'created_at',
          'event_id',
          'observed_at',
          'publisher_id',
          'app_id',
          'placement_id',
          'adapter_id',
          'adapter_name',
          'impression_id',
          'revenue_type',
          'revenue_usd',
          'revenue_currency',
          'revenue_original',
          'exchange_rate',
          'ecpm_usd',
          'country_code',
          'ad_format',
          'os',
          'is_test_mode',
          'metadata',
          'reconciliation_status',
        ],
        analytics_revenue_events_stage: [
          'id',
          'created_at',
          'event_id',
          'observed_at',
          'publisher_id',
          'app_id',
          'placement_id',
          'adapter_id',
          'adapter_name',
          'impression_id',
          'revenue_type',
          'revenue_usd',
          'revenue_currency',
          'revenue_original',
          'exchange_rate',
          'ecpm_usd',
          'country_code',
          'ad_format',
          'os',
          'is_test_mode',
          'metadata',
          'reconciliation_status',
        ],
      };

      const cols = columnsByTable[tableName] ?? [];
      return Promise.resolve({ rows: cols.map((column_name) => ({ column_name })) });
    }

    return Promise.resolve({ rows: [] });
  }),
}));

const mockedInsertMany = insertMany as jest.MockedFunction<typeof insertMany>;
const mockedQuery = query as jest.MockedFunction<typeof query>;

describe('AnalyticsService Postgres ingestion', () => {
  let service: AnalyticsService;

  beforeEach(() => {
    jest.clearAllMocks();
    mockedInsertMany.mockClear();
    mockedQuery.mockClear();
    service = new AnalyticsService({ batchSize: 1, flushIntervalMs: 0 });
  });

  afterEach(() => {
    service.shutdown();
  });

  const baseTimestamp = new Date('2025-01-01T00:00:00.000Z').toISOString();

  it('writes impressions into analytics_impressions with metadata payload', async () => {
    const event: ImpressionEventDTO = {
      event_id: '550e8400-e29b-41d4-a716-446655440000',
      timestamp: baseTimestamp,
      publisher_id: '123e4567-e89b-12d3-a456-426614174000',
      app_id: '223e4567-e89b-12d3-a456-426614174000',
      placement_id: '323e4567-e89b-12d3-a456-426614174000',
      adapter_id: '423e4567-e89b-12d3-a456-426614174000',
      adapter_name: 'AdapterOne',
      ad_unit_id: 'ca-app-pub-foo',
      ad_format: 'banner',
      country_code: 'US',
      device_type: 'phone',
      os: 'ios',
      os_version: '16.0',
      app_version: '1.0.0',
      sdk_version: '2.0.0',
      session_id: '523e4567-e89b-12d3-a456-426614174000',
      user_id: 'user123',
      request_id: '623e4567-e89b-12d3-a456-426614174000',
      bid_price_usd: 2.5,
      ecpm_usd: 3.0,
      latency_ms: 120,
      is_test_mode: false,
    };

    await service.recordImpression(event);

    expect(mockedInsertMany).toHaveBeenCalledTimes(1);
    const [table, columns, rows, options] = mockedInsertMany.mock.calls[0];
    expect(table).toBe('analytics_impressions_stage');
    expect(columns).toContain('event_id');
    expect(columns).toContain('meta');
    expect(options).toBeUndefined();
    const payload = rows[0];
    expect(payload[0]).toBe(event.event_id);
    expect(payload[1]).toBeInstanceOf(Date);
    const metaIndex = columns.indexOf('meta');
    expect(metaIndex).toBeGreaterThanOrEqual(0);
    expect(payload[metaIndex]).toMatchObject({
      app_version: event.app_version,
      sdk_version: event.sdk_version,
      bid_price_usd: event.bid_price_usd,
      ecpm_usd: event.ecpm_usd,
    });
    expect(mockedQuery).toHaveBeenCalledWith(expect.stringContaining('INSERT INTO "analytics_impressions"'));
    expect(mockedQuery).toHaveBeenCalledWith(expect.stringContaining('TRUNCATE "analytics_impressions_stage"'));
  });

  it('writes clicks into analytics_clicks with request metadata', async () => {
    const event: ClickEventDTO = {
      event_id: '650e8400-e29b-41d4-a716-446655440000',
      timestamp: baseTimestamp,
      impression_id: '550e8400-e29b-41d4-a716-446655440000',
      publisher_id: '123e4567-e89b-12d3-a456-426614174000',
      app_id: '223e4567-e89b-12d3-a456-426614174000',
      placement_id: '323e4567-e89b-12d3-a456-426614174000',
      adapter_id: '423e4567-e89b-12d3-a456-426614174000',
      adapter_name: 'AdapterOne',
      click_url: 'https://example.com/click',
      country_code: 'US',
      device_type: 'phone',
      os: 'ios',
      session_id: '523e4567-e89b-12d3-a456-426614174000',
      user_id: 'user123',
      request_id: '623e4567-e89b-12d3-a456-426614174000',
      time_to_click_ms: 2000,
      is_verified: true,
      is_test_mode: false,
    };

    await service.recordClick(event);

    const [table, , rows] = mockedInsertMany.mock.calls[0];
    expect(table).toBe('analytics_clicks_stage');
    expect(rows[0][2]).toBe(event.impression_id);
    expect(rows[0][17]).toMatchObject({ request_id: event.request_id });
    expect(mockedQuery).toHaveBeenCalledWith(expect.stringContaining('INSERT INTO "analytics_clicks"'));
  });

  it('writes revenue events into analytics_revenue_events', async () => {
    const event: RevenueEventDTO = {
      event_id: '750e8400-e29b-41d4-a716-446655440000',
      timestamp: baseTimestamp,
      publisher_id: '123e4567-e89b-12d3-a456-426614174000',
      app_id: '223e4567-e89b-12d3-a456-426614174000',
      placement_id: '323e4567-e89b-12d3-a456-426614174000',
      adapter_id: '423e4567-e89b-12d3-a456-426614174000',
      adapter_name: 'AdapterOne',
      impression_id: '550e8400-e29b-41d4-a716-446655440000',
      revenue_type: 'impression',
      revenue_usd: 1.23,
      revenue_currency: 'USD',
      revenue_original: 1.23,
      exchange_rate: 1,
      ecpm_usd: 3.21,
      country_code: 'US',
      ad_format: 'banner',
      os: 'ios',
      is_test_mode: false,
      reconciliation_status: 'pending',
    };

    await service.recordRevenue(event);

    const [table, , rows, options] = mockedInsertMany.mock.calls[0];
    expect(table).toBe('analytics_revenue_events_stage');
    expect(rows[0][9]).toBe(event.revenue_usd);
    expect(rows[0][17]).toBe(event.is_test_mode);
    expect(options).toBeUndefined();
    expect(mockedQuery).toHaveBeenCalledWith(expect.stringContaining('INSERT INTO "analytics_revenue_events"'));
  });
});
