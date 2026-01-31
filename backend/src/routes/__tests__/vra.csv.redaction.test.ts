import request from 'supertest';
import express from 'express';

// Mock auth to inject a user and bypass real auth
jest.mock('../../middleware/auth', () => ({
  authenticate: jest.fn((req, _res, next) => {
    (req as any).user = { userId: 'u1', publisherId: 'pub-1' };
    next();
  }),
}));

// Mock vraService.getDeltas to return crafted reasonCode values
jest.mock('../../services/vra/vraService', () => ({
  vraService: {
    getDeltas: jest.fn(async () => ({
      page: 1,
      pageSize: 3,
      total: 3,
      items: [
        {
          kind: 'underpay',
          amount: 1.23,
          currency: 'USD',
          reasonCode: 'contact jane.doe@example.com, token Bearer abc.def.ghi\nnew line here',
          windowStart: '2025-11-01T00:00:00Z',
          windowEnd: '2025-11-02T00:00:00Z',
          evidenceId: 'ev1',
          confidence: 0.9,
        },
        {
          kind: 'fx_mismatch',
          amount: 0,
          currency: 'USD',
          reasonCode: 'fx_band_exceeded_EUR, sk_test_1234567890, 4242424242424242',
          windowStart: '2025-11-03T00:00:00Z',
          windowEnd: '2025-11-04T00:00:00Z',
          evidenceId: 'ev2',
          confidence: 0.6,
        },
        {
          kind: 'missing',
          amount: 0,
          currency: 'USD',
          // raw JWT (three base64url segments) without Bearer prefix
          reasonCode: 'investigate jwt eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.QUJDLURFRi5HSUk.SUotS0w',
          windowStart: '2025-11-05T00:00:00Z',
          windowEnd: '2025-11-06T00:00:00Z',
          evidenceId: 'ev3',
          confidence: 0.55,
        },
      ],
    })),
  },
}));

import vraRoutes from '../../routes/vra.routes';

describe('VRA CSV export — redaction of reason_code', () => {
  let app: express.Application;

  beforeEach(() => {
    jest.clearAllMocks();
    app = express();
    app.use(express.json());
    process.env.VRA_ENABLED = 'true';
    app.use('/api/v1', vraRoutes);
  });

  it('redacts emails/tokens/JWT and strips commas/newlines from reason_code', async () => {
    const res = await request(app)
      .get('/api/v1/recon/deltas.csv')
      .expect(200)
      .expect('Content-Type', /text\/csv/);

    const body = res.text;
    const lines = body.trim().split('\n');
    expect(lines[0]).toBe('kind,amount,currency,reason_code,window_start,window_end,evidence_id,confidence');

    // First data line: email and Bearer token must be redacted; commas/newlines/tabs removed; amount formatted to 6 decimals
    const l1 = lines[1];
    expect(l1).toContain('underpay,1.230000,USD');
    expect(l1).toContain('[REDACTED_EMAIL]');
    expect(l1).toContain('Bearer [REDACTED]');
    // Ensure there are exactly 8 columns (commas inside reason removed)
    expect(l1.split(',').length).toBe(8);

    // Second data line: stripe key and long numeric redacted; commas stripped; amount formatted to 6 decimals
    const l2 = lines[2];
    expect(l2).toContain('fx_mismatch,0.000000,USD');
    expect(l2).toContain('sk_test_[REDACTED]');
    expect(l2).toMatch(/\[REDACTED_(NUMERIC|CARD)\]/);
    expect(l2.split(',').length).toBe(8);

    // Third data line: raw JWT should be redacted; amount formatted 0.000000
    const l3 = lines[3];
    expect(l3).toContain('missing,0.000000,USD');
    expect(l3).toContain('[REDACTED_JWT]');
    expect(l3.split(',').length).toBe(8);
  });
});
