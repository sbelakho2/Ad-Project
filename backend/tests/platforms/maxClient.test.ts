/**
 * Adversarial contract tests for the AppLovin MAX management client.
 * Offline: PlatformHttp is constructed with a scripted fake client per case.
 */
import { MaxClient, normalizeCountry, assertValidFloor } from '../../src/services/platforms/maxClient';
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
    platform: 'applovin_max',
    baseUrl: 'https://max.test',
    auth: () => ({ 'X-API-Key': 'key-1' }),
    client: client as never,
    sleep: async () => undefined,
  });
  return { client, http, calls };
}

const CREDS = { apiKey: 'key-1', accountId: 'acct-1' };

describe('MaxClient', () => {
  it('rejects construction without credentials', () => {
    expect(() => new MaxClient({ apiKey: '', accountId: 'x' })).toThrow(/apiKey and accountId/);
  });

  it('listAdUnits parses the documented envelope and fails closed on malformed entries', async () => {
    const { http } = harness([
      { status: 200, data: { ad_units: [{ ad_unit_id: 'u1', name: 'Banner', format: 'banner', enabled: true }] } },
    ]);
    const units = await new MaxClient(CREDS, http).listAdUnits();
    expect(units).toEqual([{ platformUnitId: 'u1', name: 'Banner', format: 'banner', enabled: true }]);

    const bad = harness([{ status: 200, data: { ad_units: [{ name: 'no id' }] } }]);
    await expect(new MaxClient(CREDS, bad.http).listAdUnits()).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('country normalization: trims, uppercases, rejects invalid codes', () => {
    expect(normalizeCountry(' ee ')).toBe('EE');
    expect(() => normalizeCountry('EST')).toThrow(/invalid country/);
    expect(() => normalizeCountry('1!')).toThrow(/invalid country/);
  });

  it('floor validation: rejects NaN/negative/out-of-range, rounds to cents', () => {
    expect(assertValidFloor(1.239)).toBe(1.24);
    expect(() => assertValidFloor(Number.NaN)).toThrow(/out of range/);
    expect(() => assertValidFloor(-1)).toThrow(/out of range/);
    expect(() => assertValidFloor(501)).toThrow(/out of range/);
  });

  it('setCountryFloor PUTs the exact documented body to the country path', async () => {
    const { http, calls } = harness([{ status: 200, data: {} }]);
    const client = new MaxClient(CREDS, http);
    await expect(client.setCountryFloor('unit/a b', 'ee', 2.5)).resolves.toEqual({ country: 'EE', floorEcpm: 2.5 });
    expect(calls[0].method).toBe('PUT');
    expect(calls[0].url).toBe('https://max.test/v1/max/ad_units/unit%2Fa%20b/floors/EE');
    expect(calls[0].data).toEqual({ floor_ecpm: 2.5 });
  });

  it('createExperiment requires an auditable single change and returns the id from the response', async () => {
    const client = new MaxClient(CREDS, harness([{ status: 200, data: { experiment_id: 'exp-9' } }]).http);
    await expect(
      client.createExperiment({ name: 'floors-ee', adUnitId: 'u1', metric: 'ecpm', variants: 2, changeDescription: 'raise EE floor to 2.0' }),
    ).resolves.toEqual({ experimentId: 'exp-9' });
    await expect(
      client.createExperiment({ name: 'x', adUnitId: 'u1', metric: 'ecpm', variants: 2, changeDescription: '   ' }),
    ).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
    await expect(
      client.createExperiment({ name: 'x', adUnitId: 'u1', metric: 'ecpm', variants: 5, changeDescription: 'raised floor' }),
    ).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('createExperiment sends an idempotency key so retries cannot duplicate experiments', async () => {
    const { http, calls } = harness([{ status: 200, data: { experiment_id: 'e' } }]);
    await new MaxClient(CREDS, http).createExperiment({ name: 'n', adUnitId: 'u', metric: 'revenue', variants: 2, changeDescription: 'c' });
    expect((calls[0].headers as Record<string, string>)['Idempotency-Key']).toBe('max-experiment:n:u');
  });

  it('getExperiment computes ecpm per variant and maps optional platform lift fields', async () => {
    const { http } = harness([
      {
        status: 200,
        data: {
          state: 'completed',
          lift_percent: 7.5,
          confidence: 0.98,
          winner_variant_id: 'v2',
          variants: [
            { variant_id: 'v1', impressions: 1000, revenue: 5 },
            { variant_id: 'v2', impressions: 0, revenue: 0 },
          ],
        },
      },
    ]);
    const result = await new MaxClient(CREDS, http).getExperiment('exp-1');
    expect(result).toMatchObject({ state: 'completed', liftPercent: 7.5, confidence: 0.98, winnerVariantId: 'v2' });
    expect(result.results[0].ecpm).toBe(5); // 5/1000*1000
    expect(result.results[1].ecpm).toBe(0); // zero impressions must not divide by zero
  });

  it('getExperiment fails closed when variants are missing or malformed', async () => {
    const { http } = harness([{ status: 200, data: { state: 'running' } }]);
    await expect(new MaxClient(CREDS, http).getExperiment('e')).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
    const bad = harness([{ status: 200, data: { state: 'running', variants: [{ variant_id: 1 }] } }]);
    await expect(new MaxClient(CREDS, bad.http).getExperiment('e')).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('unknown platform state is treated as still running (never assume completion)', async () => {
    const { http } = harness([{ status: 200, data: { state: 'weird', variants: [] } }]);
    const result = await new MaxClient(CREDS, http).getExperiment('e');
    expect(result.state).toBe('running');
  });
});
