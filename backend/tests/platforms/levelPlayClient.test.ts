/**
 * Adversarial contract tests for the LevelPlay management client (offline harness).
 */
import { LevelPlayClient, LEVELPLAY_ENDPOINTS } from '../../src/services/platforms/levelPlayClient';
import { PlatformHttp } from '../../src/services/platforms/platformHttp';

function harness(responses: Array<{ status: number; data?: unknown; headers?: Record<string, string> }>) {
  const calls: Array<Record<string, unknown>> = [];
  let i = 0;
  const client = {
    request: jest.fn(async (config: Record<string, unknown>) => {
      calls.push(config);
      const r = responses[Math.min(i, responses.length - 1)];
      i += 1;
      return { status: r.status, data: r.data, headers: r.headers ?? {} };
    }),
  };
  const http = new PlatformHttp({
    platform: 'levelplay',
    baseUrl: 'https://lp.test',
    auth: () => ({ Authorization: 'Bearer tok' }),
    client: client as never,
    sleep: async () => undefined,
  });
  return { client, http, calls };
}

const CREDS = { apiKey: 'tok', accountId: 'acct' };

describe('LevelPlayClient', () => {
  it('listWaterfall parses entries and fails closed on shape drift', async () => {
    const { http } = harness([
      {
        status: 200,
        data: {
          waterfall: [
            { entry_id: 'e1', network: 'admob', instance_name: 'ADM_1', position: 1, price_ecpm: 3.2 },
            { id: 'e2', network: 'facebook', position: 2, price_ecpm: 1.1 },
          ],
        },
      },
    ]);
    const entries = await new LevelPlayClient(CREDS, http).listWaterfall('u/1');
    expect(entries).toHaveLength(2);
    expect(entries[0]).toMatchObject({ entryId: 'e1', network: 'admob', position: 1, priceEcpm: 3.2 });
    expect(entries[1].instanceName).toBe('facebook'); // instance_name absent -> network fallback

    const bad = harness([{ status: 200, data: { waterfall: [{ entry_id: 'e', network: 'x' }] } }]);
    await expect(new LevelPlayClient(CREDS, bad.http).listWaterfall('u')).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('updateEntryPrice validates range and PATCHes the exact body with an idempotency key', async () => {
    const { http, calls } = harness([{ status: 200, data: {} }]);
    const client = new LevelPlayClient(CREDS, http);
    await expect(client.updateEntryPrice('u1', 'e1', 2.345)).resolves.toMatchObject({ priceEcpm: 2.35 });
    expect(calls[0].method).toBe('PATCH');
    expect(calls[0].url).toBe('https://lp.test/v1/adunits/u1/waterfall/e1');
    expect(calls[0].data).toEqual({ price_ecpm: 2.35 });
    expect((calls[0].headers as Record<string, string>)['Idempotency-Key']).toBe('levelplay-price:u1:e1');

    await expect(client.updateEntryPrice('u1', 'e1', 0)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
    await expect(client.updateEntryPrice('u1', 'e1', Number.POSITIVE_INFINITY)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('createEntry requires network+instance, validates price, extracts entry_id', async () => {
    const { http } = harness([{ status: 200, data: { entry_id: 'new-1' } }]);
    const client = new LevelPlayClient(CREDS, http);
    await expect(client.createEntry('u1', 'mintegral', 'MTG_1', 0.75)).resolves.toEqual({ entryId: 'new-1' });
    await expect(client.createEntry('u1', ' ', 'x', 1)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
    const missing = harness([{ status: 200, data: {} }]);
    await expect(new LevelPlayClient(CREDS, missing.http).createEntry('u1', 'n', 'i', 1)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('removeEntry DELETEs the entry path with an idempotency key', async () => {
    const { http, calls } = harness([{ status: 204, data: undefined }]);
    await new LevelPlayClient(CREDS, http).removeEntry('u1', 'e9');
    expect(calls[0].method).toBe('DELETE');
    expect(calls[0].url).toBe('https://lp.test/v1/adunits/u1/waterfall/e9');
    expect((calls[0].headers as Record<string, string>)['Idempotency-Key']).toBe('levelplay-remove:u1:e9');
  });

  it('carries the documented/verify-live endpoint markers', () => {
    expect(Object.keys(LEVELPLAY_ENDPOINTS).sort()).toEqual([
      'createEntry',
      'listWaterfall',
      'removeEntry',
      'updateEntryPrice',
    ]);
  });
});
