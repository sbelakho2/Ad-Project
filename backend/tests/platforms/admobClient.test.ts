/**
 * Adversarial contract tests for the AdMob management client (offline harness).
 */
import { AdmobClient, normalizeAccountId } from '../../src/services/platforms/admobClient';
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
    platform: 'admob',
    baseUrl: 'https://admob.test',
    auth: () => ({ Authorization: 'Bearer at' }),
    client: client as never,
    sleep: async () => undefined,
  });
  return { client, http, calls };
}

const CREDS = { accessToken: 'at', accountId: 'pub-1234567890123456' };

const GROUP = {
  mediationGroupId: 'g1',
  name: 'Interstitial group',
  format: 'interstitial',
  adUnitMappings: [
    { adUnitId: 'u1', network: 'admob', ecpm: 2.0 },
    { adUnitId: 'u2', network: 'facebook', ecpm: 1.0 },
  ],
};

describe('AdmobClient', () => {
  it('normalizes and validates account ids', () => {
    expect(normalizeAccountId('pub-1234567890123456')).toBe('accounts/pub-1234567890123456');
    expect(normalizeAccountId('accounts/pub-1234567890123456')).toBe('accounts/pub-1234567890123456');
    expect(() => normalizeAccountId('1234')).toThrow(/invalid AdMob account id/);
    expect(() => normalizeAccountId('pub-12')).toThrow(/invalid AdMob account id/);
  });

  it('listMediationGroups follows pages and fails closed on runaway pagination', async () => {
    const page1 = { mediationGroups: [GROUP], nextPageToken: 't1' };
    const page2 = { mediationGroups: [], nextPageToken: '' };
    const { http } = harness([
      { status: 200, data: page1 },
      { status: 200, data: page2 },
    ]);
    const groups = await new AdmobClient(CREDS, http).listMediationGroups();
    expect(groups).toHaveLength(1);
    expect(groups[0].mappings).toEqual([
      { adUnitId: 'u1', network: 'admob', ecpm: 2.0 },
      { adUnitId: 'u2', network: 'facebook', ecpm: 1.0 },
    ]);

    // Runaway: every page returns a token.
    const loops = Array.from({ length: 60 }, () => ({ status: 200, data: { mediationGroups: [], nextPageToken: 'again' } }));
    const runaway = harness(loops);
    await expect(new AdmobClient(CREDS, runaway.http).listMediationGroups()).rejects.toMatchObject({
      code: 'SCHEMA_INVALID',
    });
    expect(runaway.client.request).toHaveBeenCalledTimes(50); // hard page bound
  });

  it('updateMappingEcpm performs read-modify-write with the full mapping list and idempotency key', async () => {
    const { http, calls } = harness([
      { status: 200, data: { mediationGroups: [GROUP] } }, // listMediationGroups pass
      { status: 200, data: {} }, // PATCH
    ]);
    const updated = await new AdmobClient(CREDS, http).updateMappingEcpm('g1', 'u2', 1.75);
    expect(updated).toEqual({ adUnitId: 'u2', network: 'facebook', ecpm: 1.75 });
    expect(calls[1].method).toBe('PATCH');
    expect(calls[1].url).toBe('https://admob.test/v1/accounts/pub-1234567890123456/mediationGroups/g1');
    expect(calls[1].data).toEqual({
      adUnitMappings: [
        { adUnitId: 'u1', network: 'admob', ecpm: 2.0 },
        { adUnitId: 'u2', network: 'facebook', ecpm: 1.75 },
      ],
    });
    expect((calls[1].headers as Record<string, string>)['Idempotency-Key']).toBe('admob-ecpm:g1:u2:1.75');
  });

  it('updateMappingEcpm refuses unknown groups, unknown units, and out-of-range eCPMs', async () => {
    const notFound = harness([{ status: 200, data: { mediationGroups: [] } }]);
    await expect(new AdmobClient(CREDS, notFound.http).updateMappingEcpm('gX', 'u1', 1)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });

    const noUnit = harness([{ status: 200, data: { mediationGroups: [GROUP] } }]);
    await expect(new AdmobClient(CREDS, noUnit.http).updateMappingEcpm('g1', 'nope', 1)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });

    const range = harness([{ status: 200, data: { mediationGroups: [GROUP] } }]);
    await expect(new AdmobClient(CREDS, range.http).updateMappingEcpm('g1', 'u1', 0)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
    await expect(new AdmobClient(CREDS, range.http).updateMappingEcpm('g1', 'u1', 501)).rejects.toMatchObject({ code: 'SCHEMA_INVALID' });
  });

  it('401 from AdMob surfaces as non-retryable AUTH (no double spend of the OAuth quota)', async () => {
    const { http, client } = harness([{ status: 401 }]);
    await expect(new AdmobClient(CREDS, http).listMediationGroups()).rejects.toMatchObject({ code: 'AUTH', retryable: false });
    expect(client.request).toHaveBeenCalledTimes(1);
  });
});
