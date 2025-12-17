import { describe, it, expect, beforeEach, afterEach, beforeAll, afterAll, jest } from '@jest/globals';
import { SSAISessionClient } from '../../src/ssai/SSAISessionClient';
import type { SSAIConfig } from '../../src/ssai/types';

describe('SSAISessionClient (Web)', () => {
  let client: SSAISessionClient;
  const mockFetch = jest.fn();
  const originalFetch = global.fetch;

  const mockConfig: SSAIConfig = {
    endpoint: 'https://ssai.example.com',
    tenantId: 'tenant-123',
    contentUrl: 'https://content.example.com/video.m3u8',
    format: 'hls',
    capabilities: {
      maxResolution: { width: 1920, height: 1080 },
      codecs: ['avc1', 'mp4a'],
      supportsHdr: false,
      supportsDolby: false,
    },
    timeoutSeconds: 60,
    debug: false,
  };

  const mockSessionResponse = {
    sessionId: 'session-abc123',
    manifestUrl: 'https://ssai.example.com/manifest/session-abc123/master.m3u8',
    trackingUrl: 'https://ssai.example.com/track/session-abc123',
    expiresAt: new Date(Date.now() + 3600_000).toISOString(),
    sessionToken: 'token-xyz',
  };

  beforeAll(() => {
    (global as any).fetch = mockFetch;
  });

  afterAll(() => {
    (global as any).fetch = originalFetch;
  });

  beforeEach(() => {
    jest.clearAllMocks();
    client = new SSAISessionClient({ enableBatching: false });
  });

  afterEach(async () => {
    if (client.getSession()) {
      mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({}) });
      await client.endSession();
    }
  });

  it('creates a session and parses expiresAt as Date', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockSessionResponse),
    });

    const session = await client.createSession(mockConfig);

    expect(session.sessionId).toBe('session-abc123');
    expect(session.contentUrl).toBe(mockConfig.contentUrl);
    expect(session.expiresAt).toBeInstanceOf(Date);
    expect(mockFetch).toHaveBeenCalledWith(
      'https://ssai.example.com/v1/ssai/session',
      expect.objectContaining({ method: 'POST' })
    );
  });

  it('tracks an impression to trackingUrl/event with session token', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockSessionResponse),
    });
    await client.createSession(mockConfig);

    mockFetch.mockResolvedValueOnce({ ok: true });

    // With batching disabled, events are sent immediately.
    client.trackImpression('pod-1', 'slot-1', 'creative-1', 0);

    const calls = mockFetch.mock.calls;
    const lastCall = calls[calls.length - 1];
    expect(String(lastCall[0])).toContain(
      'https://ssai.example.com/track/session-abc123/event'
    );
    expect((lastCall[1] as any).headers['X-Session-Token']).toBe('token-xyz');

    const body = JSON.parse(String((lastCall[1] as any).body));
    expect(body.type).toBe('impression');
  });
});
