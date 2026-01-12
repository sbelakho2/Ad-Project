import { init, setConsent, requestAd, Errors } from '../src';
import { state as mswState, timeoutHandler } from './msw/handlers';
import { server } from './msw/server';
import { http, HttpResponse } from 'msw';

describe('Web SDK', () => {
  beforeEach(() => {
    // reset MSW state
    mswState.lastAuctionBody = null;
  });

  it('throws if not initialized', async () => {
    await expect(requestAd({ placement: 'home', adType: 'banner' })).rejects.toMatchObject({ code: 'INIT_REQUIRED' });
  });

  it('initializes, sets consent, and requests ad successfully', async () => {
    init({ endpoint: 'https://ads.example.test', timeoutMs: 2000, sdkVersion: '0.1.0' });
    setConsent({ gdprApplies: true, tcfConsent: 'COabcd...', usPrivacy: '1YNN' });

    const resp = await requestAd({ placement: 'home', adType: 'banner', width: 300, height: 250 });
    expect(resp.fill).toBe(true);
    expect(resp.creative?.html).toContain('Ad');

    // Ensure consent propagated
    expect(mswState.lastAuctionBody?.consent).toMatchObject({ gdprApplies: true, tcfConsent: 'COabcd...', usPrivacy: '1YNN' });
    // Ensure meta present
    expect(mswState.lastAuctionBody?.meta?.sdk?.name).toBe('@rivalapex/web-sdk');
  });

  it('times out when server is slow', async () => {
    server.use(timeoutHandler);
    init({ endpoint: 'https://ads.example.test', timeoutMs: 50, sdkVersion: '0.1.0' });
    await expect(requestAd({ placement: 'slow', adType: 'banner' })).rejects.toMatchObject({ code: 'TIMEOUT' });
  });

  it('retries on 5xx and eventually succeeds within maxRetries', async () => {
    let count = 0;
    server.use(
      http.post('*/auction', async ({ request }) => {
        count += 1;
        if (count <= 2) {
          return HttpResponse.json({ error: 'temporary' }, { status: 502 });
        }
        const json = await request.json();
        mswState.lastAuctionBody = json as any;
        return HttpResponse.json({ requestId: 'ok', fill: true, creative: { id: 'x', html: '<div>Ad</div>' } });
      })
    );
    init({ endpoint: 'https://ads.example.test', timeoutMs: 2000, maxRetries: 3, sdkVersion: '0.1.0' });
    const resp = await requestAd({ placement: 'retry', adType: 'banner' });
    expect(resp.fill).toBe(true);
    expect(count).toBeGreaterThanOrEqual(3);
  });

  it('does not retry on 4xx and surfaces BAD_RESPONSE', async () => {
    server.use(
      http.post('*/auction', async () => {
        return HttpResponse.json({ error: 'bad request' }, { status: 400 });
      })
    );
    init({ endpoint: 'https://ads.example.test', timeoutMs: 500, maxRetries: 5, sdkVersion: '0.1.0' });
    await expect(requestAd({ placement: 'noretry', adType: 'banner' })).rejects.toMatchObject({ code: 'BAD_RESPONSE' });
  });

  it('retries on network error and then succeeds', async () => {
    let count = 0;
    server.use(
      http.post('*/auction', async ({ request }) => {
        count += 1;
        if (count <= 2) {
          return HttpResponse.error();
        }
        const json = await request.json();
        mswState.lastAuctionBody = json as any;
        return HttpResponse.json({ requestId: 'ok2', fill: true, creative: { id: 'y', html: '<div>Ad</div>' } });
      })
    );
    init({ endpoint: 'https://ads.example.test', timeoutMs: 2000, maxRetries: 3, retryBackoffBaseMs: 10, retryJitterMs: 5, sdkVersion: '0.1.0' });
    const resp = await requestAd({ placement: 'net', adType: 'banner' });
    expect(resp.fill).toBe(true);
    expect(count).toBeGreaterThanOrEqual(3);
  });
});
