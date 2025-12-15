import { describe, it, expect, beforeEach, afterEach, jest } from '@jest/globals';
import { SSAISessionClient } from '../../../src/ssai/SSAISessionClient';
import { SSAIConfig, SSAISession, SSAIAdBreak, SSAIAd } from '../../../src/ssai/types';

// Mock fetch
const mockFetch = jest.fn();
global.fetch = mockFetch as any;

describe('SSAISessionClient', () => {
  let client: SSAISessionClient;
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
    timeoutMs: 5000,
    debug: false,
  };

  const mockSession: SSAISession = {
    sessionId: 'session-abc123',
    manifestUrl: 'https://ssai.example.com/manifest/session-abc123/master.m3u8',
    trackingUrl: 'https://ssai.example.com/track/session-abc123',
    expiresAt: new Date(Date.now() + 3600000),
    sessionToken: 'token-xyz',
    adBreaks: [
      {
        podId: 'pod-1',
        type: 'preroll',
        position: 0,
        startTime: 0,
        duration: 30,
        plannedSlots: 2,
        filledSlots: 2,
        ads: [
          {
            slotId: 'slot-1',
            creativeId: 'creative-1',
            duration: 15,
            startOffset: 0,
            skipOffset: 5,
            vastUrl: 'https://vast.example.com/ad1.xml',
            verificationScripts: [],
          },
          {
            slotId: 'slot-2',
            creativeId: 'creative-2',
            duration: 15,
            startOffset: 15,
            skipOffset: null,
            vastUrl: 'https://vast.example.com/ad2.xml',
            verificationScripts: [],
          },
        ],
      },
    ],
  };

  beforeEach(() => {
    jest.clearAllMocks();
    client = new SSAISessionClient();
  });

  afterEach(() => {
    client.destroy();
  });

  describe('createSession', () => {
    it('should create a session successfully', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(mockSession),
      });

      const session = await client.createSession(mockConfig);

      expect(session).toEqual(mockSession);
      expect(mockFetch).toHaveBeenCalledWith(
        'https://ssai.example.com/v1/ssai/session',
        expect.objectContaining({
          method: 'POST',
          headers: expect.objectContaining({
            'Content-Type': 'application/json',
          }),
        })
      );
    });

    it('should handle session creation failure', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 500,
        statusText: 'Internal Server Error',
      });

      await expect(client.createSession(mockConfig)).rejects.toThrow();
    });

    it('should retry on network failure', async () => {
      mockFetch
        .mockRejectedValueOnce(new Error('Network error'))
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve(mockSession),
        });

      const session = await client.createSession(mockConfig);

      expect(session).toEqual(mockSession);
      expect(mockFetch).toHaveBeenCalledTimes(2);
    });
  });

  describe('trackEvent', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(mockSession),
      });
      await client.createSession(mockConfig);
      mockFetch.mockClear();
    });

    it('should track impression event', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true });

      client.trackImpression('pod-1', 'slot-1', 'creative-1', 0);
      await client.flush();

      expect(mockFetch).toHaveBeenCalledWith(
        expect.stringContaining('/events'),
        expect.objectContaining({
          method: 'POST',
        })
      );
    });

    it('should deduplicate events', async () => {
      mockFetch.mockResolvedValue({ ok: true });

      client.trackImpression('pod-1', 'slot-1', 'creative-1', 0);
      client.trackImpression('pod-1', 'slot-1', 'creative-1', 0);
      await client.flush();

      // Should only send one batch with one event
      const calls = mockFetch.mock.calls;
      const lastCall = calls[calls.length - 1];
      const body = JSON.parse(lastCall[1].body);
      
      expect(body.events.filter((e: any) => e.type === 'impression')).toHaveLength(1);
    });

    it('should track quartile progress', async () => {
      mockFetch.mockResolvedValue({ ok: true });

      client.trackProgress('pod-1', 'slot-1', 'creative-1', 0, 0);
      client.trackProgress('pod-1', 'slot-1', 'creative-1', 3.75, 25);
      client.trackProgress('pod-1', 'slot-1', 'creative-1', 7.5, 50);
      client.trackProgress('pod-1', 'slot-1', 'creative-1', 11.25, 75);
      client.trackProgress('pod-1', 'slot-1', 'creative-1', 15, 100);
      
      await client.flush();

      expect(mockFetch).toHaveBeenCalled();
    });
  });

  describe('endSession', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(mockSession),
      });
      await client.createSession(mockConfig);
      mockFetch.mockClear();
    });

    it('should end session and cleanup', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true });

      await client.endSession();

      expect(mockFetch).toHaveBeenCalledWith(
        expect.stringContaining('/session-abc123/end'),
        expect.objectContaining({
          method: 'POST',
        })
      );
    });
  });

  describe('session validity', () => {
    it('should report session as valid when not expired', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(mockSession),
      });
      await client.createSession(mockConfig);

      expect(client.isSessionValid).toBe(true);
    });

    it('should report session as invalid when expired', async () => {
      const expiredSession = {
        ...mockSession,
        expiresAt: new Date(Date.now() - 1000),
      };
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(expiredSession),
      });
      await client.createSession(mockConfig);

      expect(client.isSessionValid).toBe(false);
    });
  });
});
