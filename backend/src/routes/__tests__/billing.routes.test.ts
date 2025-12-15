/**
 * Unit tests for billing routes
 * Uses mocked dependencies - no database required
 */

import request from 'supertest';
import express from 'express';

// Mock pg and Redis BEFORE any imports that use them
jest.mock('pg', () => {
  const mClient = {
    connect: jest.fn(),
    query: jest.fn(),
    end: jest.fn(),
  };
  const mPool = {
    connect: jest.fn(),
    query: jest.fn(),
    end: jest.fn(),
    on: jest.fn(),
  };
  return { Pool: jest.fn(() => mPool), Client: jest.fn(() => mClient) };
});

jest.mock('redis', () => ({
  createClient: jest.fn(() => ({
    connect: jest.fn(),
    get: jest.fn(),
    set: jest.fn(),
    del: jest.fn(),
    quit: jest.fn(),
    on: jest.fn(),
  })),
}));

jest.mock('stripe', () => {
  return jest.fn().mockImplementation(() => ({
    billing: { meterEvents: { create: jest.fn() } },
  }));
});

// Mock middleware and dependencies
jest.mock('../../middleware/auth', () => ({
  authenticate: jest.fn((req, res, next) => {
    req.user = {
      userId: 'test-user-123',
      organizationId: 'org-123'
    };
    next();
  }),
  authorize: jest.fn(() => (req: any, res: any, next: any) => next()),
}));

jest.mock('../../utils/featureFlags', () => ({
  requireFeature: jest.fn(() => (req: any, res: any, next: any) => next()),
}));

jest.mock('../../controllers/billing.controller', () => ({
  getCurrentUsage: jest.fn((req, res) => {
    res.json({
      current_period: { start: '2025-01-01', end: '2025-01-31' },
      usage: { impressions: 1000, clicks: 50, video_starts: 10 },
      limits: { impressions: 10000, clicks: 500, video_starts: 100 }
    });
  }),
  createPortalSession: jest.fn((_req, res) => {
    res.json({
      success: true,
      url: 'https://billing.stripe.test/portal-session',
    });
  }),
  getBillingPolicy: jest.fn((_req, res) => {
    res.json({
      success: true,
      data: {
        version: 'policy-test',
        primaryRail: { id: 'stripe' },
        fallbackRails: [],
      },
    });
  }),
  listInvoices: jest.fn((req, res) => {
    res.json({
      invoices: [
        { id: 'inv-1', amount: 10000, status: 'paid', period: '2025-01' }
      ],
      pagination: { page: 1, limit: 10, total: 1 }
    });
  }),
  getInvoice: jest.fn((req, res) => {
    res.json({
      id: req.params.id,
      number: 'INV-2025-001',
      amount: 10000,
      status: 'paid',
      items: []
    });
  }),
  getInvoicePDF: jest.fn((req, res) => {
    res.setHeader('Content-Type', 'application/pdf');
    res.send(Buffer.from('PDF content'));
  }),
  requestMigration: jest.fn((req, res) => {
    res.status(202).json({
      success: true,
      data: {
        requestId: 'migration-req-123',
        channel: req.body?.channel ?? 'sandbox',
        notes: req.body?.notes ?? 'mock request',
      },
    });
  }),
  reconcileBilling: jest.fn((req, res) => {
    res.json({
      status: 'completed',
      discrepancies: []
    });
  }),
  calculatePlatformFee: jest.fn((req, res) => {
    res.json({ success: true, data: { gross_revenue_cents: req.body?.gross_revenue_cents ?? 0, tier: { id: 'starter' } } });
  }),
  getPlatformTiers: jest.fn((req, res) => {
    res.json({ success: true, data: [{ id: 'starter', name: 'Tier 0 — Starter' }] });
  }),
}));

import billingRoutes from '../billing.routes';

describe('Billing Routes', () => {
  let app: express.Application;

  beforeAll(() => {
    app = express();
    app.use(express.json());
    app.use('/api/v1/billing', billingRoutes);
  });

  describe('GET /api/v1/billing/policy', () => {
    it('returns the billing policy snapshot without auth', async () => {
      const response = await request(app)
        .get('/api/v1/billing/policy')
        .expect(200);

      expect(response.body).toEqual(
        expect.objectContaining({
          success: true,
          data: expect.objectContaining({ version: 'policy-test' }),
        })
      );
    });
  });

  describe('GET /api/v1/billing/usage/current', () => {
    it('should return current usage data', async () => {
      const response = await request(app)
        .get('/api/v1/billing/usage/current')
        .expect(200);

      expect(response.body).toHaveProperty('current_period');
      expect(response.body).toHaveProperty('usage');
      expect(response.body.usage).toHaveProperty('impressions');
    });
  });

  describe('GET /api/v1/billing/invoices', () => {
    it('should return list of invoices', async () => {
      const response = await request(app)
        .get('/api/v1/billing/invoices')
        .expect(200);

      expect(response.body).toHaveProperty('invoices');
      expect(response.body).toHaveProperty('pagination');
      expect(Array.isArray(response.body.invoices)).toBe(true);
    });
  });

  describe('GET /api/v1/billing/invoices/:id', () => {
    it('should return specific invoice', async () => {
      const response = await request(app)
        .get('/api/v1/billing/invoices/in_test123')
        .expect(200);

      expect(response.body).toHaveProperty('id');
      expect(response.body).toHaveProperty('number');
    });
  });

  describe('GET /api/v1/billing/invoices/:id/pdf', () => {
    it('should return PDF with correct content type', async () => {
      const response = await request(app)
        .get('/api/v1/billing/invoices/in_test123/pdf')
        .expect(200);

      expect(response.headers['content-type']).toBe('application/pdf');
    });
  });

  describe('POST /api/v1/billing/reconcile', () => {
    it('should return 403 since reconciliation is operator-only', async () => {
      const response = await request(app)
        .post('/api/v1/billing/reconcile')
        .send({
          organization_id: 'org-123',
          period_start: '2025-11-01T00:00:00Z',
          period_end: '2025-11-30T23:59:59Z',
        })
        .set('Idempotency-Key', 'reconcile-test-key-123')
        .expect(403);

      expect(response.body).toHaveProperty('error');
    });
  });

  describe('POST /api/v1/billing/platform-fees/calculate', () => {
    it('returns platform fee calculation payload', async () => {
      const response = await request(app)
        .post('/api/v1/billing/platform-fees/calculate')
        .send({ gross_revenue_cents: 2500000 })
        .expect(200);

      expect(response.body).toHaveProperty('success', true);
      expect(response.body.data).toHaveProperty('gross_revenue_cents', 2500000);
      expect(response.body.data).toHaveProperty('tier');
    });
  });

  describe('GET /api/v1/billing/platform-fees/tiers', () => {
    it('returns tier metadata', async () => {
      const response = await request(app)
        .get('/api/v1/billing/platform-fees/tiers')
        .expect(200);

      expect(response.body).toHaveProperty('success', true);
      expect(Array.isArray(response.body.data)).toBe(true);
      expect(response.body.data[0]).toHaveProperty('id');
    });
  });
});
