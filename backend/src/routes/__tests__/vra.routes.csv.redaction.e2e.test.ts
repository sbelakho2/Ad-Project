import request from 'supertest';
import express from 'express';

// Mock auth to inject a user and bypass real auth
jest.mock('../../middleware/auth', () => ({
  authenticate: jest.fn((req, _res, next) => {
    (req as any).user = { userId: 'u1', publisherId: 'pub-1' };
    next();
  }),
}));

jest.mock('../../services/vra/vraService');

import vraRoutes from '../../routes/vra.routes';

describe('VRA Routes — CSV redaction E2E', () => {
  let app: express.Application;

  beforeEach(() => {
    app = express();
    app.use(express.json());
    process.env.VRA_ENABLED = 'true';
    app.use('/api/v1', vraRoutes);
  });

  it('CSV response redacts emails, bearer tokens, and long numerics', async () => {
    const { vraService } = jest.requireMock('../../services/vra/vraService');
    (vraService.getDeltas as jest.Mock).mockResolvedValueOnce({
      items: [
        {
          kind: 'underpay',
          amount: 1,
          currency: 'USD',
          reasonCode: 'email: test.user@example.com',
          windowStart: '2025-11-01 00:00:00',
          windowEnd: '2025-11-02 00:00:00',
          evidenceId: 'ev-a',
          confidence: 0.9,
        },
        {
          kind: 'fx_mismatch',
          amount: 0,
          currency: 'USD',
          reasonCode: 'token Bearer abc.def.ghi',
          windowStart: '2025-11-01 00:00:00',
          windowEnd: '2025-11-02 00:00:00',
          evidenceId: 'ev-b',
          confidence: 0.6,
        },
        {
          kind: 'timing_lag',
          amount: 0,
          currency: 'USD',
          reasonCode: 'digits 4242424242424242 inside',
          windowStart: '2025-11-01 00:00:00',
          windowEnd: '2025-11-02 00:00:00',
          evidenceId: 'ev-c',
          confidence: 0.5,
        },
      ],
      page: 1,
      pageSize: 500,
      total: 3,
    });

    const res = await request(app)
      .get('/api/v1/recon/deltas.csv?from=2025-11-01T00:00:00Z&to=2025-11-02T00:00:00Z&page=1&page_size=500')
      .expect(200);

    const csv = res.text || '';
    // Email redacted
    expect(csv).toContain('[REDACTED_EMAIL]');
    expect(csv).not.toContain('test.user@example.com');
    // Bearer redacted
    expect(csv).toContain('Bearer [REDACTED]');
    expect(csv).not.toContain('abc.def.ghi');
    // Long numerics redacted
    expect(csv).toMatch(/\[REDACTED_(NUMERIC|CARD)\]/);
    expect(csv).not.toContain('4242424242424242');
  });
});
