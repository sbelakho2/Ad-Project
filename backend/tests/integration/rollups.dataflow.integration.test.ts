/**
 * Rollups + DB Dataflow Integration Tests
 *
 * Purpose: validate the end-to-end pipeline:
 *   HTTP ingest -> in-memory buffers -> Postgres stage tables -> fact tables -> daily rollups
 *
 * These tests are intentionally DB-backed (guarded by FORCE_DB_SETUP=true).
 */

import request from 'supertest';
import type { Application } from 'express';
import { jest } from '@jest/globals';
import { Client } from 'pg';

// Integration suite needs the real pg driver rather than the shared Jest manual mock.
jest.mock('pg', () => jest.requireActual('pg'));

import { createTestApp } from '../helpers/testApp';
import analyticsService from '../../src/services/analyticsService';
import { runAnalyticsRollups } from '../../src/services/analyticsRollupService';

const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

describe('DB dataflow: analytics ingest -> rollups', () => {
  let app: Application;
  let client: Client;

  beforeAll(async () => {
    app = createTestApp();

    if (!process.env.DATABASE_URL) {
      throw new Error('DATABASE_URL is required for DB-backed integration tests');
    }
    client = new Client({ connectionString: process.env.DATABASE_URL });
    await client.connect();
  });

  afterAll(async () => {
    analyticsService.shutdown();
    await client.end();
  });

  beforeEach(async () => {
    // Keep tests isolated and deterministic.
    await client.query('TRUNCATE analytics_impressions_stage, analytics_clicks_stage, analytics_revenue_events_stage');
    await client.query('TRUNCATE analytics_impressions, analytics_clicks, analytics_revenue_events');
    await client.query('TRUNCATE analytics_impression_rollups, analytics_click_rollups, analytics_revenue_rollups, analytics_metrics_rollups');
  });

  it('counts clicks from analytics_clicks in rollups (CTR dataflow)', async () => {
    const publisherId = '123e4567-e89b-12d3-a456-426614174000';
    const appId = '223e4567-e89b-12d3-a456-426614174000';
    const placementId = '323e4567-e89b-12d3-a456-426614174000';
    const adapterId = '423e4567-e89b-12d3-a456-426614174000';

    const observedAtIso = '2026-01-01T12:00:00.000Z';

    const impressionEventId = '550e8400-e29b-41d4-a716-446655440000';
    const clickEventId = '650e8400-e29b-41d4-a716-446655440000';

    await request(app)
      .post('/api/v1/analytics/events/impressions')
      .send({
        events: [
          {
            event_id: impressionEventId,
            timestamp: observedAtIso,
            publisher_id: publisherId,
            app_id: appId,
            placement_id: placementId,
            adapter_id: adapterId,
            adapter_name: 'TestAdapter',
            ad_unit_id: 'ca-app-pub-123456',
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
            latency_ms: 150,
            is_test_mode: false,
          },
        ],
      })
      .expect(202);

    await request(app)
      .post('/api/v1/analytics/events/clicks')
      .send({
        events: [
          {
            event_id: clickEventId,
            timestamp: observedAtIso,
            impression_id: impressionEventId,
            publisher_id: publisherId,
            app_id: appId,
            placement_id: placementId,
            adapter_id: adapterId,
            adapter_name: 'TestAdapter',
            click_url: 'https://example.com/click',
            country_code: 'US',
            device_type: 'phone',
            os: 'ios',
            session_id: '523e4567-e89b-12d3-a456-426614174000',
            user_id: 'user123',
            request_id: '623e4567-e89b-12d3-a456-426614174000',
            time_to_click_ms: 2500,
            is_verified: true,
            is_test_mode: false,
          },
        ],
      })
      .expect(202);

    // Force synchronous flush to DB (bypasses the periodic timer).
    await analyticsService.flushAll();

    // Small wait: mergeStageIntoTarget truncates stage tables; ensure everything commits.
    await sleep(50);

    const impressionResult = await client.query(
      'SELECT COUNT(*)::int AS n FROM analytics_impressions WHERE publisher_id = $1',
      [publisherId]
    );
    const clickResult = await client.query(
      'SELECT COUNT(*)::int AS n FROM analytics_clicks WHERE publisher_id = $1',
      [publisherId]
    );

    expect(impressionResult).toBeDefined();
    expect(clickResult).toBeDefined();

    const impressionRows = (impressionResult as any)?.rows ?? [];
    const clickRows = (clickResult as any)?.rows ?? [];

    expect(impressionRows[0]?.n).toBe(1);
    expect(clickRows[0]?.n).toBe(1);

    // Run rollups for the day.
    await runAnalyticsRollups({
      startInclusive: new Date('2026-01-01T00:00:00.000Z'),
      endExclusive: new Date('2026-01-02T00:00:00.000Z'),
    });

    const rollupResult = await client.query(
      `
        SELECT click_count::bigint AS clicks, impression_count::bigint AS impressions
        FROM analytics_metrics_rollups
        WHERE bucket_date = '2026-01-01'
          AND publisher_id = $1
          AND country_code = 'US'
        LIMIT 1
      `,
      [publisherId]
    );

    expect(rollupResult).toBeDefined();
    const rollupRows = (rollupResult as any)?.rows ?? [];

    expect(rollupRows.length).toBe(1);
    expect(Number(rollupRows[0].impressions)).toBe(1);
    expect(Number(rollupRows[0].clicks)).toBe(1);
  });
});
