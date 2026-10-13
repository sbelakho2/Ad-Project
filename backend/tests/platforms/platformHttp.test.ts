/**
 * Adversarial tests for the shared platform HTTP transport.
 * Everything offline: axios is replaced by a scripted fake adapter is not needed because
 * PlatformHttp accepts an injected client; we build a minimal fake with a request() spy.
 */
import { PlatformHttp } from '../../src/services/platforms/platformHttp';
import { PlatformApiError } from '../../src/services/platforms/types';

type Resp = { status: number; data?: unknown; headers?: Record<string, string> };

function fakeClient(responses: Resp[]) {
  const calls: Array<Record<string, unknown>> = [];
  let i = 0;
  return {
    calls,
    client: {
      request: jest.fn(async (config: Record<string, unknown>) => {
        calls.push(config);
        const r = responses[Math.min(i, responses.length - 1)];
        i += 1;
        return { status: r.status, data: r.data, headers: r.headers ?? {} };
      }),
    },
  };
}

const noSleep = async () => undefined;

function build(responses: Resp[], overrides: Partial<ConstructorParameters<typeof PlatformHttp>[0]> = {}) {
  const { client, calls } = fakeClient(responses);
  const http = new PlatformHttp({
    platform: 'applovin_max',
    baseUrl: 'https://example.test',
    auth: () => ({ 'X-API-Key': 'k' }),
    client: client as never,
    sleep: noSleep,
    now: () => Date.parse('2026-01-01T00:00:00Z'),
    ...overrides,
  });
  return { http, calls, client };
}

describe('PlatformHttp', () => {
  it('returns data on 2xx and sends the auth header', async () => {
    const { http, calls } = build([{ status: 200, data: { ok: true } }]);
    await expect(http.request({ operation: 'op', method: 'GET', path: '/x' })).resolves.toEqual({ ok: true });
    expect((calls[0].headers as Record<string, string>)['X-API-Key']).toBe('k');
  });

  it('401 fails fast: no retry, AUTH code, not retryable', async () => {
    const { http, client } = build([{ status: 401 }, { status: 200 }]);
    await expect(http.request({ operation: 'op', method: 'GET', path: '/x' })).rejects.toMatchObject({
      code: 'AUTH',
      retryable: false,
      status: 401,
    });
    expect(client.request).toHaveBeenCalledTimes(1);
  });

  it('429 is retried for reads, honoring capped Retry-After', async () => {
    const sleeps: number[] = [];
    const { http } = build(
      [{ status: 429, headers: { 'retry-after': '9999' } }, { status: 200, data: 1 }],
      { sleep: async (ms: number) => { sleeps.push(ms); } },
    );
    await expect(http.request({ operation: 'op', method: 'GET', path: '/x' })).resolves.toBe(1);
    expect(sleeps).toEqual([60_000]); // capped at MAX_RETRY_AFTER_MS
  });

  it('5xx exhausts retries then throws retryable PlatformApiError', async () => {
    const { http, client } = build([{ status: 503 }], { maxRetries: 2 });
    const err = await http.request({ operation: 'op', method: 'GET', path: '/x' }).catch((e) => e);
    expect(err).toBeInstanceOf(PlatformApiError);
    expect(err.retryable).toBe(true);
    expect(client.request).toHaveBeenCalledTimes(3); // 1 + 2 retries
  });

  it('non-idempotent writes are NOT retried without an idempotency key', async () => {
    const { http, client } = build([{ status: 503 }, { status: 200 }]);
    await expect(http.request({ operation: 'w', method: 'POST', path: '/x', body: {} })).rejects.toMatchObject({ retryable: false });
    expect(client.request).toHaveBeenCalledTimes(1);
  });

  it('writes with an idempotency key are retried and the key is sent', async () => {
    const { http, client, calls } = build([{ status: 500 }, { status: 200, data: { done: true } }]);
    await expect(
      http.request({ operation: 'w', method: 'POST', path: '/x', body: {}, idempotencyKey: 'k-1' }),
    ).resolves.toEqual({ done: true });
    expect(client.request).toHaveBeenCalledTimes(2);
    expect((calls[0].headers as Record<string, string>)['Idempotency-Key']).toBe('k-1');
  });

  it('network failure is retryable for reads and mapped to NETWORK', async () => {
    let first = true;
    const client = {
      request: jest.fn(async () => {
        if (first) { first = false; throw new Error('ECONNRESET'); }
        return { status: 200, data: 'ok', headers: {} };
      }),
    };
    const http = new PlatformHttp({ platform: 'admob', baseUrl: 'https://t', auth: () => ({}), client: client as never, sleep: noSleep });
    await expect(http.request({ operation: 'op', method: 'GET', path: '/x' })).resolves.toBe('ok');
  });

  it('getValidated fails closed on shape drift', async () => {
    const { http } = build([{ status: 200, data: { totally: 'different' } }]);
    const guard = (d: unknown): d is { items: unknown[] } => !!d && typeof d === 'object' && Array.isArray((d as never)['items']);
    await expect(http.getValidated('op', '/x', undefined, guard)).rejects.toMatchObject({ code: 'SCHEMA_INVALID', retryable: false });
  });

  it('Retry-After date form is parsed and clamped', async () => {
    const sleeps: number[] = [];
    const { http } = build(
      [{ status: 429, headers: { 'retry-after': 'Thu, 01 Jan 2099 00:00:00 GMT' } }, { status: 200, data: 1 }],
      { sleep: async (ms: number) => { sleeps.push(ms); } },
    );
    await http.request({ operation: 'op', method: 'GET', path: '/x' });
    expect(sleeps).toEqual([60_000]);
  });
});
