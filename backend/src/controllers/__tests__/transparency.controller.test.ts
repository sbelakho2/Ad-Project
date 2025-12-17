/* eslint-disable no-empty */
import request from 'supertest';
import type { Application, NextFunction, Request, Response } from 'express';
import { generateKeyPairSync } from 'crypto';

// Mock auth to inject a default user unless overridden per-test
jest.mock('../../middleware/auth', () => ({
  authenticate: jest.fn((req: Request & { user?: any }, _res: Response, next: NextFunction) => {
    // Allow tests to bypass auth by sending header 'noauth: 1'
    if (!('noauth' in req.headers)) {
      req.user = { publisherId: 'pub-1', userId: 'user-1' };
    }
    next();
  }),
  authorize: jest.fn(() => (_req: Request, _res: Response, next: NextFunction) => next()),
}));

// Mock Postgres query helper
const queryMock = jest.fn();
jest.mock('../../utils/postgres', () => ({
  query: (q: string, params?: ReadonlyArray<unknown>) => queryMock(q, params),
}));

import { createTestApp } from '../../__tests__/helpers/testApp';

const AUCTION_ID = '11111111-1111-4111-8111-111111111111';

const asResult = (rows: any[]) => ({ rows });

function resetEnv() {
  delete process.env.TRANSPARENCY_PUBLIC_KEY_BASE64;
  delete process.env.TRANSPARENCY_KEY_ID;
  process.env.TRANSPARENCY_API_ENABLED = 'true';
}

describe('Transparency Controller — verification API', () => {
  let app: Application;

  beforeAll(() => {
    app = createTestApp();
  });

  beforeEach(() => {
    jest.clearAllMocks();
    queryMock.mockReset();
    resetEnv();
  });

  it('GET /transparency/keys falls back to env when table empty', async () => {
    process.env.TRANSPARENCY_PUBLIC_KEY_BASE64 = '-----BEGIN PUBLIC KEY-----\nMFYwEAYHKoZIzj0CAQYFK4EEAAoDQgAEXAMPLEKEYLINE\n-----END PUBLIC KEY-----\n';
    process.env.TRANSPARENCY_KEY_ID = 'env-key-1';
    // make signer table empty
    queryMock.mockResolvedValueOnce(asResult([]));

    const res = await request(app)
      .get('/api/v1/transparency/keys')
      .set('Authorization', 'Bearer test')
      .expect(200);

    expect(res.body.count).toBe(1);
    expect(res.body.data[0]).toMatchObject({ key_id: 'env-key-1', algo: 'ed25519' });
  });

  it('GET /transparency/keys returns 200 even when DB query fails (public/no auth)', async () => {
    process.env.TRANSPARENCY_PUBLIC_KEY_BASE64 = '-----BEGIN PUBLIC KEY-----\nMFYwEAYHKoZIzj0CAQYFK4EEAAoDQgAEXAMPLEKEYLINE\n-----END PUBLIC KEY-----\n';
    process.env.TRANSPARENCY_KEY_ID = 'env-key-1';

    queryMock.mockRejectedValueOnce(new Error('relation "transparency_signer_keys" does not exist'));

    const res = await request(app)
      .get('/api/v1/transparency/keys')
      .expect(200);

    expect(res.body.count).toBe(1);
    expect(res.body.data[0]).toMatchObject({ key_id: 'env-key-1', algo: 'ed25519' });
  });

  it('GET /transparency/auctions/:id/verify → not_applicable when signature missing', async () => {
    // 1) auction row without signature
    queryMock.mockImplementationOnce(async () => asResult([
      {
        auction_id: AUCTION_ID,
        observed_at: new Date().toISOString(),
        publisher_id: 'pub-1',
        winner_source: 'alpha',
        winner_bid_ecpm: 1.23,
        winner_currency: 'USD',
        winner_reason: 'highest_bid',
        aletheia_fee_bp: 150,
        sample_bps: 250,
        integrity_algo: 'ed25519',
        integrity_key_id: 'key-1',
        integrity_signature: null,
      },
    ]));
    // 2) candidates query
    queryMock.mockImplementationOnce(async () => asResult([]));

    const res = await request(app)
      .get(`/api/v1/transparency/auctions/${AUCTION_ID}/verify`)
      .set('Authorization', 'Bearer test')
      .expect(200);

    expect(res.body.status).toBe('not_applicable');
  });

  it('GET /transparency/auctions/:id/verify → unknown_key when key not found and no env fallback', async () => {
    // auction with signature and key id
    const { privateKey, publicKey } = generateKeyPairSync('ed25519');
    const timestamp = new Date().toISOString();

    // Build canonical payload like controller does
    const payload = {
      auction: {
        auction_id: AUCTION_ID,
        publisher_id: 'pub-1',
        timestamp,
        winner_source: 'alpha',
        winner_bid_ecpm: 1.23,
        winner_currency: 'USD',
        winner_reason: 'highest_bid',
        sample_bps: 250,
      },
      candidates: [
        { source: 'alpha', bid_ecpm: 1.23, status: 'winner' },
        { source: 'beta', bid_ecpm: 1.1, status: 'loss' },
      ],
    };

    const canonical = canonicalString(payload);
    // Node crypto.sign(null, ...) API is used in code; use it directly here
    const signature = require('crypto').sign(null, Buffer.from(canonical, 'utf8'), privateKey).toString('base64');

    // Query 1: auction row
    queryMock.mockImplementationOnce(async () => asResult([
      {
        auction_id: AUCTION_ID,
        observed_at: timestamp,
        publisher_id: 'pub-1',
        winner_source: 'alpha',
        winner_bid_ecpm: 1.23,
        winner_currency: 'USD',
        winner_reason: 'highest_bid',
        aletheia_fee_bp: 150,
        sample_bps: 250,
        integrity_algo: 'ed25519',
        integrity_key_id: 'key-unknown',
        integrity_signature: signature,
      },
    ]));
    // Query 2: candidates
    queryMock.mockImplementationOnce(async () => asResult([
      { source: 'alpha', bid_ecpm: 1.23, status: 'winner' },
      { source: 'beta', bid_ecpm: 1.1, status: 'loss' },
    ]));
    // Query 3: keys (none)
    queryMock.mockImplementationOnce(async () => asResult([]));

    const res = await request(app)
      .get(`/api/v1/transparency/auctions/${AUCTION_ID}/verify`)
      .set('Authorization', 'Bearer test')
      .expect(200);

    expect(res.body.status).toBe('unknown_key');
  });

  it('GET /transparency/auctions/:id/verify → pass with valid signature and key', async () => {
    const { privateKey, publicKey } = generateKeyPairSync('ed25519');
    const timestamp = new Date().toISOString();
    const payload = {
      auction: {
        auction_id: AUCTION_ID,
        publisher_id: 'pub-1',
        timestamp,
        winner_source: 'alpha',
        winner_bid_ecpm: 1.23,
        winner_currency: 'USD',
        winner_reason: 'highest_bid',
        sample_bps: 250,
      },
      candidates: [
        { source: 'alpha', bid_ecpm: 1.23, status: 'winner' },
        { source: 'beta', bid_ecpm: 1.1, status: 'loss' },
      ],
    };
    const canonical = canonicalString(payload);
    const signature = require('crypto').sign(null, Buffer.from(canonical, 'utf8'), privateKey).toString('base64');

    // Auction row
    queryMock.mockImplementationOnce(async () => asResult([
      {
        auction_id: AUCTION_ID,
        observed_at: timestamp,
        publisher_id: 'pub-1',
        winner_source: 'alpha',
        winner_bid_ecpm: 1.23,
        winner_currency: 'USD',
        winner_reason: 'highest_bid',
        aletheia_fee_bp: 150,
        sample_bps: 250,
        integrity_algo: 'ed25519',
        integrity_key_id: 'key-1',
        integrity_signature: signature,
      },
    ]));
    // Candidates
    queryMock.mockImplementationOnce(async () => asResult([
      { source: 'alpha', bid_ecpm: 1.23, status: 'winner' },
      { source: 'beta', bid_ecpm: 1.1, status: 'loss' },
    ]));
    // Keys (active)
    const pem = publicKey.export({ type: 'spki', format: 'pem' }).toString();
    queryMock.mockImplementationOnce(async () => asResult([
      { key_id: 'key-1', algo: 'ed25519', public_key_base64: pem, active: 1 },
    ]));

    const res = await request(app)
      .get(`/api/v1/transparency/auctions/${AUCTION_ID}/verify`)
      .set('Authorization', 'Bearer test')
      .expect(200);

    expect(res.body.status).toBe('pass');
    expect(res.body.key_id).toBe('key-1');
    expect(typeof res.body.canonical).toBe('string');
  });

  it('GET /transparency/auctions/:id/verify → 401 when unauthenticated', async () => {
    // Provide auction row so controller can scope-check before doing more work
    queryMock.mockImplementationOnce(async () => asResult([
      {
        auction_id: AUCTION_ID,
        observed_at: new Date().toISOString(),
        publisher_id: 'pub-1',
      },
    ]));

    // Suppress auth by sending a header our mock checks
    const res = await request(app)
      .get(`/api/v1/transparency/auctions/${AUCTION_ID}/verify`)
      .set('noauth', '1')
      .expect(401);
    expect(res.body.error || res.body.reason).toBeDefined();
  });

  it('GET /transparency/auctions/:id/verify → 503 when feature disabled', async () => {
    process.env.TRANSPARENCY_API_ENABLED = 'false';

    const res = await request(app)
      .get(`/api/v1/transparency/auctions/${AUCTION_ID}/verify`)
      .set('Authorization', 'Bearer test')
      .expect(503);

    expect(res.body.error).toBeDefined();
  });
});

// Minimal copy of canonicalString to avoid importing writer in tests (keeps parity with controller)
function canonicalString(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((entry) => canonicalString(entry)).join(',')}]`;
  }
  if (value && typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b));
    const inner = entries
      .map(([key, val]) => `${JSON.stringify(key)}:${canonicalString(val)}`)
      .join(',');
    return `{${inner}}`;
  }
  if (typeof value === 'string') return JSON.stringify(value);
  if (typeof value === 'number') return Number.isFinite(value) ? value.toString() : 'null';
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  return 'null';
}
