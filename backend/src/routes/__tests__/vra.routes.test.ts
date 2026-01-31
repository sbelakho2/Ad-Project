import request from 'supertest';
import express from 'express';

// Mock auth to inject a user and bypass real auth
jest.mock('../../middleware/auth', () => ({
  authenticate: jest.fn((req, _res, next) => {
    (req as any).user = { userId: 'u1', publisherId: 'pub-1' };
    next();
  }),
}));

// Mock Postgres helper usage in query/insertMany
jest.mock('../../utils/postgres', () => ({
  query: jest.fn(async (sql: string) => {
    const q = String(sql).toLowerCase();
    if (q.includes('count(')) {
      return { rows: [{ total: '1' }] };
    }
    return { rows: [] };
  }),
  insertMany: jest.fn(async () => {}),
}));

import vraRoutes from '../../routes/vra.routes';

describe('VRA Routes', () => {
  let app: express.Application;

  beforeEach(() => {
    // Reset mocked DB helpers between tests so mockResolvedValueOnce queues
    // never leak across tests (which can cause hangs/incorrect rows).
    const { query, insertMany } = jest.requireMock('../../utils/postgres');
    const queryImpl = (query as jest.Mock).getMockImplementation();
    (query as jest.Mock).mockReset();
    if (queryImpl) (query as jest.Mock).mockImplementation(queryImpl);
    const insertManyImpl = (insertMany as jest.Mock).getMockImplementation();
    (insertMany as jest.Mock).mockReset();
    if (insertManyImpl) (insertMany as jest.Mock).mockImplementation(insertManyImpl);

    app = express();
    app.use(express.json());
    // Reset env flags each test
    delete process.env.VRA_ENABLED;
    delete process.env.VRA_SHADOW_ONLY;
    // Set a generous read-only rate limit by default so tests don't flake on 429s
    process.env.TRANSPARENCY_RATE_LIMIT_RPM_DEFAULT = '1000';
    app.use('/api/v1', vraRoutes);
  });

  it('returns 404 when VRA feature disabled', async () => {
    process.env.VRA_ENABLED = 'false';
    await request(app).get('/api/v1/recon/overview').expect(404);
  });

  it('CSV export returns 404 when VRA feature disabled', async () => {
    process.env.VRA_ENABLED = 'false';
    await request(app).get('/api/v1/recon/deltas.csv').expect(404);
  });

  it('returns overview payload when enabled (CH empty)', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app).get('/api/v1/recon/overview').expect(200);
    expect(res.body).toHaveProperty('success', true);
    expect(res.body.data).toHaveProperty('coveragePercent');
    expect(res.body.data).toHaveProperty('totals');
  });

  it('POST /recon/disputes responds with 202 in shadow mode', async () => {
    process.env.VRA_ENABLED = 'true';
    process.env.VRA_SHADOW_ONLY = 'true';
    const { insertMany } = jest.requireMock('../../utils/postgres');
    (insertMany as jest.Mock).mockClear();
    const res = await request(app)
      .post('/api/v1/recon/disputes')
      .send({ delta_ids: ['e1','e2'], network: 'unity', contact: 'ops@example.com' })
      .expect(202);
    expect(res.body).toHaveProperty('success', true);
    expect(res.body.data).toHaveProperty('shadow', true);
    // Ensure no writes performed in shadow mode
    expect(insertMany).not.toHaveBeenCalled();
  });

  it('GET /proofs/revenue_digest with bad month returns 400', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/proofs/revenue_digest?month=202511')
      .expect(400);
    expect(res.body).toHaveProperty('success', false);
  });

  it('GET /proofs/revenue_digest returns 404 when digest not found', async () => {
    process.env.VRA_ENABLED = 'true';
    const { query } = jest.requireMock('../../utils/postgres');
    // Mock monthly digest select to return empty list
    (query as jest.Mock).mockResolvedValueOnce({ rows: [] });
    const res = await request(app)
      .get('/api/v1/proofs/revenue_digest?month=2025-11')
      .expect(404);
    expect(res.body).toHaveProperty('success', false);
    expect(res.body).toHaveProperty('error');
  });

  it('GET /proofs/revenue_digest returns 200 with digest payload', async () => {
    process.env.VRA_ENABLED = 'true';
    const { query } = jest.requireMock('../../utils/postgres');
    // Monthly digest select returns one row
    (query as jest.Mock).mockResolvedValueOnce({ rows: [{
      month: '2025-11',
      digest: 'deadbeef',
      sig: 'cafebabe',
      coverage_pct: '97.5',
      notes: 'ok',
    }] });
    const res = await request(app)
      .get('/api/v1/proofs/revenue_digest?month=2025-11')
      .expect(200);
    expect(res.body).toHaveProperty('success', true);
    expect(res.body.data).toMatchObject({ month: '2025-11', digest: 'deadbeef' });
  });

  it('GET /recon/deltas.csv returns CSV with header and proper content type', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas.csv')
      .expect(200);
    expect(res.headers['content-type']).toContain('text/csv');
    // first line should be the header
    const text = res.text || '';
    expect(text.split('\n')[0]).toBe('kind,amount,currency,reason_code,window_start,window_end,evidence_id,confidence');
  });

  it('GET /recon/deltas.csv includes Content-Disposition filename with from/to when provided', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas.csv?from=2025-11-01T00:00:00Z&to=2025-11-02T00:00:00Z')
      .expect(200);
    const cd = res.headers['content-disposition'] || '';
    expect(cd).toContain('attachment;');
    // Suffix contains _<from>_to_<to> in YYYY-MM-DD form
    expect(cd).toMatch(/recon_deltas_.*2025-11-01.*_to_.*2025-11-02.*\.csv"?$/);
  });

  it('GET /recon/deltas.csv omits _from_to_ suffix when only one window bound provided', async () => {
    process.env.VRA_ENABLED = 'true';
    // Only from provided
    let res = await request(app)
      .get('/api/v1/recon/deltas.csv?from=2025-11-01T00:00:00Z')
      .expect(200);
    let cd = res.headers['content-disposition'] || '';
    expect(cd).toContain('attachment;');
    expect(cd).not.toMatch(/_to_/);

    // Only to provided
    res = await request(app)
      .get('/api/v1/recon/deltas.csv?to=2025-11-02T00:00:00Z')
      .expect(200);
    cd = res.headers['content-disposition'] || '';
    expect(cd).toContain('attachment;');
    expect(cd).not.toMatch(/_to_/);
  });

  it('GET /recon/deltas.csv redacts PII/secrets in reason_code', async () => {
    process.env.VRA_ENABLED = 'true';
    const { query } = jest.requireMock('../../utils/postgres');
    // First call: count -> already covered by default mock. Second call: list rows
    (query as jest.Mock).mockResolvedValueOnce({ rows: [{ total: '1' }] });
    ;(query as jest.Mock).mockResolvedValueOnce({ rows: [{
      kind: 'underpay',
      amount: '12.340000',
      currency: 'USD',
      reason_code: 'contact: alice@example.com token: Bearer abc.def.ghi digits: 4242424242424242',
      window_start: '2025-11-01 00:00:00',
      window_end: '2025-11-02 00:00:00',
      evidence_id: 'ev1',
      confidence: '0.91',
    }] });

    const res = await request(app).get('/api/v1/recon/deltas.csv').expect(200);
    const lines = (res.text || '').split('\n');
    // header + 1 row
    expect(lines.length).toBeGreaterThanOrEqual(2);
    const row = lines[1] || '';
    expect(row).toContain('[REDACTED_EMAIL]');
    expect(row).toContain('Bearer [REDACTED]');
    expect(row).toMatch(/\[REDACTED_(NUMERIC|CARD)\]/);
  });

  it('GET /recon/deltas.csv escapes quotes/newlines/commas in reason_code for stable CSV', async () => {
    process.env.VRA_ENABLED = 'true';
    const { query } = jest.requireMock('../../utils/postgres');
    // First call: count
    (query as jest.Mock).mockResolvedValueOnce({ rows: [{ total: '1' }] });
    // Second call: list row with messy reason_code
    (query as jest.Mock).mockResolvedValueOnce({ rows: [{
      kind: 'underpay',
      amount: '1.230000',
      currency: 'USD',
      reason_code: 'line1, with comma\nline2 with "quote"',
      window_start: '2025-11-01 00:00:00',
      window_end: '2025-11-02 00:00:00',
      evidence_id: 'ev-xyz',
      confidence: '0.75',
    }] });

    const res = await request(app).get('/api/v1/recon/deltas.csv').expect(200);
    const lines = (res.text || '').split('\n');
    expect(lines[0]).toBe('kind,amount,currency,reason_code,window_start,window_end,evidence_id,confidence');
    const row = lines[1] || '';
    // No raw newlines or commas inside reason field; quotes should be doubled
    expect(row).not.toMatch(/\n/);
    expect(row.split(',')).toHaveLength(8);
    expect(row).toContain('quote""');
  });

  it('GET /recon/deltas.csv validates params and returns 400 on bad min_conf', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas.csv?min_conf=2') // invalid (>1)
      .expect(400);
    expect(res.body).toHaveProperty('success', false);
    expect(res.body).toHaveProperty('error');
  });

  it('GET /recon/deltas.csv rate limits (429) when exceeding RO limiter', async () => {
    // Set low RPM to trigger limiter quickly
    process.env.VRA_ENABLED = 'true';
    process.env.TRANSPARENCY_RATE_LIMIT_RPM_DEFAULT = '1';

    // Build a fresh app instance to pick up env-based limiter setting
    app = express();
    app.use(express.json());
    app.use('/api/v1', vraRoutes);

    const res1 = await request(app).get('/api/v1/recon/deltas.csv');
    const res2 = await request(app).get('/api/v1/recon/deltas.csv');
    // In a tight test window the limiter may trigger on the first or second call depending on timing.
    // Accept either order but require that at least one request is rate-limited (429).
    expect([200, 429]).toContain(res1.status);
    expect([200, 429]).toContain(res2.status);
    expect([res1.status, res2.status]).toContain(429);
  });

  it('GET /recon/deltas returns 400 on invalid kind', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas?kind=not_a_kind')
      .expect(400);
    expect(res.body).toHaveProperty('success', false);
    expect(res.body).toHaveProperty('error');
  });

  it('GET /recon/deltas.csv returns 400 on inverted window (from > to)', async () => {
    process.env.VRA_ENABLED = 'true';
    await request(app)
      .get('/api/v1/recon/deltas.csv?from=2025-11-03T00:00:00Z&to=2025-11-02T00:00:00Z')
      .expect(400);
  });

  it('GET /recon/deltas returns 400 on invalid page_size (>500)', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas?page_size=1000')
      .expect(400);
    expect(res.body).toHaveProperty('success', false);
  });

  it('GET /recon/deltas accepts page_size=500 boundary', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas?page_size=500')
      .expect(200);
    expect(res.body).toHaveProperty('success', true);
    expect(res.body).toHaveProperty('pageSize', 500);
  });

  it('GET /recon/deltas returns 400 on invalid page (0)', async () => {
    process.env.VRA_ENABLED = 'true';
    const res = await request(app)
      .get('/api/v1/recon/deltas?page=0')
      .expect(400);
    expect(res.body).toHaveProperty('success', false);
  });
});
