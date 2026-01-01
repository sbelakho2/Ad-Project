/**
 * Analytics Integration Tests
 * 
 * Tests event ingestion endpoints for analytics Postgres pipeline
 */

import request from 'supertest';
import { Application } from 'express';
import { createTestApp } from '../helpers/testApp';
import analyticsService from '../../services/analyticsService';

describe('Analytics Event Ingestion API', () => {
  let app: Application;

  beforeAll(() => {
    app = createTestApp();
  });

  afterAll(() => {
    // Cleanup service interval
    analyticsService.shutdown();
  });

  describe('POST /api/v1/analytics/events/impressions', () => {
    it('should accept valid impression events', async () => {
      const events = [
        {
          event_id: '550e8400-e29b-41d4-a716-446655440000',
          timestamp: new Date().toISOString(),
          publisher_id: '123e4567-e89b-12d3-a456-426614174000',
          app_id: '223e4567-e89b-12d3-a456-426614174000',
          placement_id: '323e4567-e89b-12d3-a456-426614174000',
          adapter_id: '423e4567-e89b-12d3-a456-426614174000',
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
      ];

      const response = await request(app)
        .post('/api/v1/analytics/events/impressions')
        .send({ events })
        .expect(202);

      expect(response.body.success).toBe(true);
      expect(response.body.count).toBe(1);
    });

    it('should reject invalid impression data', async () => {
      const events = [
        {
          impression_id: 'not-a-uuid',
          // Missing required fields
        },
      ];

      await request(app)
        .post('/api/v1/analytics/events/impressions')
        .send({ events })
        .expect(400);
    });

    it('should reject non-array events', async () => {
      await request(app)
        .post('/api/v1/analytics/events/impressions')
        .send({ events: 'not-an-array' })
        .expect(400);
    });
  });

  describe('POST /api/v1/analytics/events/clicks', () => {
    it('should accept valid click events', async () => {
      const events = [
        {
          event_id: '650e8400-e29b-41d4-a716-446655440000',
          timestamp: new Date().toISOString(),
          impression_id: '550e8400-e29b-41d4-a716-446655440000',
          publisher_id: '123e4567-e89b-12d3-a456-426614174000',
          app_id: '223e4567-e89b-12d3-a456-426614174000',
          placement_id: '323e4567-e89b-12d3-a456-426614174000',
          adapter_id: '423e4567-e89b-12d3-a456-426614174000',
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
      ];

      const response = await request(app)
        .post('/api/v1/analytics/events/clicks')
        .send({ events })
        .expect(202);

      expect(response.body.success).toBe(true);
      expect(response.body.count).toBe(1);
    });

    it('should reject invalid click data', async () => {
      const events = [
        {
          click_id: 'invalid',
        },
      ];

      await request(app)
        .post('/api/v1/analytics/events/clicks')
        .send({ events })
        .expect(400);
    });
  });

  describe('POST /api/v1/analytics/events/revenue', () => {
    it('should accept valid revenue events', async () => {
      const events = [
        {
          event_id: '750e8400-e29b-41d4-a716-446655440000',
          timestamp: new Date().toISOString(),
          publisher_id: '123e4567-e89b-12d3-a456-426614174000',
          app_id: '223e4567-e89b-12d3-a456-426614174000',
          placement_id: '323e4567-e89b-12d3-a456-426614174000',
          adapter_id: '423e4567-e89b-12d3-a456-426614174000',
          adapter_name: 'TestAdapter',
          impression_id: '550e8400-e29b-41d4-a716-446655440000',
          revenue_type: 'impression',
          revenue_usd: 2.5,
          revenue_currency: 'USD',
          revenue_original: 2.5,
          exchange_rate: 1.0,
          ecpm_usd: 3.0,
          country_code: 'US',
          ad_format: 'banner',
          os: 'ios',
          is_test_mode: false,
          reconciliation_status: 'pending',
        },
      ];

      const response = await request(app)
        .post('/api/v1/analytics/events/revenue')
        .send({ events })
        .expect(202);

      expect(response.body.success).toBe(true);
      expect(response.body.count).toBe(1);
    });

    it('should reject negative revenue', async () => {
      const events = [
        {
          event_id: '750e8400-e29b-41d4-a716-446655440000',
          timestamp: new Date().toISOString(),
          publisher_id: '123e4567-e89b-12d3-a456-426614174000',
          app_id: '223e4567-e89b-12d3-a456-426614174000',
          placement_id: '323e4567-e89b-12d3-a456-426614174000',
          adapter_id: '423e4567-e89b-12d3-a456-426614174000',
          adapter_name: 'TestAdapter',
          impression_id: '550e8400-e29b-41d4-a716-446655440000',
          revenue_type: 'impression',
          revenue_usd: -5.0, // Invalid negative value
          revenue_currency: 'USD',
          revenue_original: -5.0,
          exchange_rate: 1.0,
          ecpm_usd: 3.0,
          country_code: 'US',
          ad_format: 'banner',
          os: 'ios',
          is_test_mode: false,
          reconciliation_status: 'pending',
        },
      ];

      await request(app)
        .post('/api/v1/analytics/events/revenue')
        .send({ events })
        .expect(400);
    });
  });

  describe('GET /api/v1/analytics/buffer-stats', () => {
    it('should require authentication', async () => {
      await request(app)
        .get('/api/v1/analytics/buffer-stats')
        .set('noauth', '1')
        .expect(401);
    });
  });
});
