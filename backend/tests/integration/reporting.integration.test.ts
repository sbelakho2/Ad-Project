/**
 * Reporting Integration Tests
 *
 * Exercises the reporting endpoints end-to-end against the Postgres-backed
 * analytics read model. These tests previously required ClickHouse, but now
 * verify the same flows using the replica-backed Postgres fixtures.
 */

import { jest } from '@jest/globals';
import request from 'supertest';
import { Pool } from 'pg';
import { Application } from 'express';
import { createTestApp } from '../helpers/testApp';
import {
  setupTestDatabase,
  teardownTestDatabase,
  cleanDatabase,
} from '../helpers/testDatabase';
import {
  createTestPublisher,
  createTestUser,
  seedReportingData,
} from '../helpers/testFixtures';

jest.mock('pg', () => jest.requireActual('pg'));

describe('Reporting API', () => {
  let pool: Pool;
  let app: Application;
  let authToken: string;
  let publisherId: string;

  const authedGet = (path: string) =>
    request(app)
      .get(path)
      .set('noauth', '1')
      .set('Authorization', `Bearer ${authToken}`);

  beforeAll(async () => {
    pool = await setupTestDatabase();
    app = createTestApp();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool);

    // Setup test data and authenticate
    const publisher = await createTestPublisher(pool);
    publisherId = publisher.id;

    const user = await createTestUser(pool, publisherId, {
      password: 'password123',
    });

    await seedReportingData(pool, publisherId);

    // Login to get auth token
    const loginResponse = await request(app)
      .post('/api/v1/auth/login')
      .send({
        email: user.email,
        password: user.password,
      });

    authToken = loginResponse.body.data.token;
  });

  describe('GET /api/v1/reporting/overview', () => {
    it('should return revenue overview stats', async () => {
      const response = await authedGet('/api/v1/reporting/overview').expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('totalRevenue');
      expect(response.body.data).toHaveProperty('totalImpressions');
      expect(response.body.data).toHaveProperty('totalClicks');
      expect(response.body.data).toHaveProperty('ecpm');
      expect(response.body.data).toHaveProperty('ctr');
      expect(response.body.data).toHaveProperty('fillRate');
      expect(response.body.data).toHaveProperty('period');
    });

    it('should accept date range parameters', async () => {
      const startDate = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString();
      const endDate = new Date().toISOString();

      const response = await authedGet('/api/v1/reporting/overview')
        .query({ startDate, endDate })
        .expect(200);

      expect(response.body.data.period.startDate).toBe(startDate);
      expect(response.body.data.period.endDate).toBe(endDate);
    });

    it('should require authentication', async () => {
      await request(app)
        .get('/api/v1/reporting/overview')
        .set('noauth', '1')
        .expect(401);
    });
  });

  describe('GET /api/v1/reporting/timeseries', () => {
    it('should return time series data', async () => {
      const response = await authedGet('/api/v1/reporting/timeseries').expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('series');
      expect(response.body.data).toHaveProperty('granularity');
      expect(Array.isArray(response.body.data.series)).toBe(true);
    });

    it('should support granularity parameter', async () => {
      const response = await authedGet('/api/v1/reporting/timeseries')
        .query({ granularity: 'hour' })
        .expect(200);

      expect(response.body.data.granularity).toBe('hour');
    });

    it('should reject invalid granularity', async () => {
      await authedGet('/api/v1/reporting/timeseries')
        .query({ granularity: 'invalid' })
        .expect(400);
    });
  });

  describe('GET /api/v1/reporting/adapters', () => {
    it('should return adapter performance breakdown', async () => {
      const response = await authedGet('/api/v1/reporting/adapters').expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('adapters');
      expect(Array.isArray(response.body.data.adapters)).toBe(true);
    });
  });

  describe('GET /api/v1/reporting/countries', () => {
    it('should return country breakdown', async () => {
      const response = await authedGet('/api/v1/reporting/countries').expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('countries');
      expect(Array.isArray(response.body.data.countries)).toBe(true);
    });

    it('should support limit parameter', async () => {
      const response = await authedGet('/api/v1/reporting/countries')
        .query({ limit: '5' })
        .expect(200);

      expect(response.body.data.countries.length).toBeLessThanOrEqual(5);
    });

    it('should reject invalid limit', async () => {
      await authedGet('/api/v1/reporting/countries')
        .query({ limit: 100 }) // Max is 50
        .expect(400);
    });
  });

  describe('GET /api/v1/reporting/top-apps', () => {
    it('should return top performing apps', async () => {
      const response = await authedGet('/api/v1/reporting/top-apps').expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('apps');
      expect(Array.isArray(response.body.data.apps)).toBe(true);
    });
  });

  describe('GET /api/v1/reporting/realtime', () => {
    it('should return real-time statistics', async () => {
      const response = await authedGet('/api/v1/reporting/realtime').expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('lastHourImpressions');
      expect(response.body.data).toHaveProperty('lastHourRevenue');
      expect(response.body.data).toHaveProperty('activeAdapters');
      expect(response.body.data).toHaveProperty('timestamp');
    });
  });
});
