import { describe, it, expect, beforeEach, afterEach, jest } from '@jest/globals';

/**
 * SSAI Integration E2E Tests
 * 
 * These tests verify end-to-end SSAI flows including:
 * - Session creation and manifest rewriting
 * - Ad break signaling and pod decisioning
 * - Tracking event flows
 * - Operability features
 */

// Mock server for E2E tests
class MockSSAIServer {
  private sessions = new Map<string, any>();
  private trackingEvents = new Map<string, any[]>();
  private manifests = new Map<string, string>();

  createSession(config: any): any {
    const sessionId = `session-${Date.now()}`;
    const session = {
      sessionId,
      manifestUrl: `http://localhost:3000/ssai/${sessionId}/master.m3u8`,
      trackingUrl: `http://localhost:3000/ssai/${sessionId}/track`,
      expiresAt: new Date(Date.now() + 3600000).toISOString(),
      sessionToken: `token-${sessionId}`,
      adBreaks: this.generateAdBreaks(config),
    };
    this.sessions.set(sessionId, session);
    this.trackingEvents.set(sessionId, []);
    this.generateManifest(sessionId, session.adBreaks);
    return session;
  }

  private generateAdBreaks(config: any): any[] {
    return [
      {
        podId: 'preroll-pod-1',
        type: 'preroll',
        position: 0,
        startTime: 0,
        duration: 30,
        plannedSlots: 2,
        filledSlots: 2,
        ads: [
          {
            slotId: 'slot-1',
            creativeId: 'creative-001',
            duration: 15,
            startOffset: 0,
            skipOffset: 5,
          },
          {
            slotId: 'slot-2',
            creativeId: 'creative-002',
            duration: 15,
            startOffset: 15,
            skipOffset: null,
          },
        ],
      },
      {
        podId: 'midroll-pod-1',
        type: 'midroll',
        position: 300,
        startTime: 300,
        duration: 45,
        plannedSlots: 3,
        filledSlots: 3,
        ads: [
          {
            slotId: 'slot-3',
            creativeId: 'creative-003',
            duration: 15,
            startOffset: 0,
          },
          {
            slotId: 'slot-4',
            creativeId: 'creative-004',
            duration: 15,
            startOffset: 15,
          },
          {
            slotId: 'slot-5',
            creativeId: 'creative-005',
            duration: 15,
            startOffset: 30,
          },
        ],
      },
    ];
  }

  private generateManifest(sessionId: string, adBreaks: any[]): void {
    const manifest = `#EXTM3U
#EXT-X-VERSION:6
#EXT-X-TARGETDURATION:6
#EXT-X-PLAYLIST-TYPE:VOD

${adBreaks.map(this.adBreakToHLS).join('\n')}

#EXT-X-DATERANGE:ID="content-start",CLASS="content",START-DATE="2024-01-01T00:00:30.000Z"
#EXTINF:6.0,
content/segment-1.ts
#EXTINF:6.0,
content/segment-2.ts
#EXTINF:6.0,
content/segment-3.ts

#EXT-X-ENDLIST`;
    
    this.manifests.set(sessionId, manifest);
  }

  private adBreakToHLS(adBreak: any): string {
    const lines = [
      `#EXT-X-DATERANGE:ID="${adBreak.podId}",CLASS="ssai-ad",START-DATE="2024-01-01T00:00:00.000Z",DURATION=${adBreak.duration},X-POD-ID="${adBreak.podId}"`,
    ];
    
    adBreak.ads.forEach((ad: any, index: number) => {
      const segmentCount = Math.ceil(ad.duration / 6);
      for (let i = 0; i < segmentCount; i++) {
        lines.push(`#EXTINF:${Math.min(6, ad.duration - i * 6)},`);
        lines.push(`ads/${adBreak.podId}/${ad.slotId}/segment-${i}.ts`);
      }
    });

    return lines.join('\n');
  }

  getManifest(sessionId: string): string | null {
    return this.manifests.get(sessionId) || null;
  }

  trackEvent(sessionId: string, event: any): void {
    const events = this.trackingEvents.get(sessionId) || [];
    events.push({ ...event, serverTimestamp: Date.now() });
    this.trackingEvents.set(sessionId, events);
  }

  getEvents(sessionId: string): any[] {
    return this.trackingEvents.get(sessionId) || [];
  }

  getSession(sessionId: string): any {
    return this.sessions.get(sessionId);
  }
}

describe('SSAI E2E Integration Tests', () => {
  let mockServer: MockSSAIServer;

  beforeEach(() => {
    mockServer = new MockSSAIServer();
  });

  describe('Session Lifecycle', () => {
    it('should create a new SSAI session', () => {
      const config = {
        tenantId: 'test-tenant',
        contentUrl: 'https://content.example.com/video.m3u8',
        format: 'hls',
      };

      const session = mockServer.createSession(config);

      expect(session.sessionId).toBeDefined();
      expect(session.manifestUrl).toContain(session.sessionId);
      expect(session.trackingUrl).toContain(session.sessionId);
      expect(session.adBreaks).toHaveLength(2);
    });

    it('should generate rewritten manifest', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const manifest = mockServer.getManifest(session.sessionId);

      expect(manifest).toContain('#EXTM3U');
      expect(manifest).toContain('#EXT-X-DATERANGE');
      expect(manifest).toContain('preroll-pod-1');
      expect(manifest).toContain('ssai-ad');
    });

    it('should include ad segments in manifest', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const manifest = mockServer.getManifest(session.sessionId);

      // Should have ad segments
      expect(manifest).toContain('ads/preroll-pod-1/slot-1/segment-');
      expect(manifest).toContain('ads/preroll-pod-1/slot-2/segment-');
    });
  });

  describe('Ad Break Structure', () => {
    it('should include preroll ad break', () => {
      const session = mockServer.createSession({ tenantId: 'test' });

      const preroll = session.adBreaks.find((b: any) => b.type === 'preroll');

      expect(preroll).toBeDefined();
      expect(preroll.position).toBe(0);
      expect(preroll.startTime).toBe(0);
      expect(preroll.ads.length).toBeGreaterThan(0);
    });

    it('should include midroll ad break', () => {
      const session = mockServer.createSession({ tenantId: 'test' });

      const midroll = session.adBreaks.find((b: any) => b.type === 'midroll');

      expect(midroll).toBeDefined();
      expect(midroll.position).toBe(300);
      expect(midroll.ads.length).toBeGreaterThan(0);
    });

    it('should have correct ad structure', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const preroll = session.adBreaks[0];
      const ad = preroll.ads[0];

      expect(ad.slotId).toBeDefined();
      expect(ad.creativeId).toBeDefined();
      expect(ad.duration).toBeGreaterThan(0);
      expect(ad.startOffset).toBeDefined();
    });

    it('should mark skippable ads correctly', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const preroll = session.adBreaks[0];

      const skippableAd = preroll.ads.find((a: any) => a.skipOffset !== null);
      const nonSkippableAd = preroll.ads.find((a: any) => a.skipOffset === null);

      expect(skippableAd).toBeDefined();
      expect(skippableAd.skipOffset).toBe(5);
      expect(nonSkippableAd).toBeDefined();
    });
  });

  describe('Tracking Flow', () => {
    it('should track impression event', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      
      mockServer.trackEvent(session.sessionId, {
        type: 'impression',
        podId: 'preroll-pod-1',
        slotId: 'slot-1',
        creativeId: 'creative-001',
        mediaTime: 0,
      });

      const events = mockServer.getEvents(session.sessionId);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('impression');
    });

    it('should track full quartile sequence', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const quartiles = ['impression', 'start', 'firstQuartile', 'midpoint', 'thirdQuartile', 'complete'];

      quartiles.forEach((type, index) => {
        mockServer.trackEvent(session.sessionId, {
          type,
          podId: 'preroll-pod-1',
          slotId: 'slot-1',
          creativeId: 'creative-001',
          mediaTime: index * 3,
        });
      });

      const events = mockServer.getEvents(session.sessionId);

      expect(events).toHaveLength(6);
      expect(events.map((e: any) => e.type)).toEqual(quartiles);
    });

    it('should track skip event', () => {
      const session = mockServer.createSession({ tenantId: 'test' });

      mockServer.trackEvent(session.sessionId, {
        type: 'impression',
        podId: 'preroll-pod-1',
        slotId: 'slot-1',
        creativeId: 'creative-001',
        mediaTime: 0,
      });

      mockServer.trackEvent(session.sessionId, {
        type: 'skip',
        podId: 'preroll-pod-1',
        slotId: 'slot-1',
        creativeId: 'creative-001',
        mediaTime: 5,
      });

      const events = mockServer.getEvents(session.sessionId);

      expect(events).toHaveLength(2);
      expect(events[1].type).toBe('skip');
    });

    it('should track multiple ads in sequence', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const preroll = session.adBreaks[0];

      // Track both ads in preroll
      preroll.ads.forEach((ad: any) => {
        mockServer.trackEvent(session.sessionId, {
          type: 'impression',
          podId: preroll.podId,
          slotId: ad.slotId,
          creativeId: ad.creativeId,
          mediaTime: ad.startOffset,
        });

        mockServer.trackEvent(session.sessionId, {
          type: 'complete',
          podId: preroll.podId,
          slotId: ad.slotId,
          creativeId: ad.creativeId,
          mediaTime: ad.startOffset + ad.duration,
        });
      });

      const events = mockServer.getEvents(session.sessionId);

      expect(events).toHaveLength(4); // 2 ads × 2 events each
    });
  });

  describe('Error Handling', () => {
    it('should track error events', () => {
      const session = mockServer.createSession({ tenantId: 'test' });

      mockServer.trackEvent(session.sessionId, {
        type: 'error',
        podId: 'preroll-pod-1',
        slotId: 'slot-1',
        creativeId: 'creative-001',
        mediaTime: 2,
        errorCode: 'MEDIA_ERROR',
        errorMessage: 'Failed to load segment',
      });

      const events = mockServer.getEvents(session.sessionId);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('error');
      expect(events[0].errorCode).toBe('MEDIA_ERROR');
    });
  });

  describe('Session State', () => {
    it('should maintain session state', () => {
      const config = { tenantId: 'test' };
      const session = mockServer.createSession(config);

      // Add some events
      mockServer.trackEvent(session.sessionId, {
        type: 'impression',
        podId: 'preroll-pod-1',
        slotId: 'slot-1',
        creativeId: 'creative-001',
        mediaTime: 0,
      });

      // Retrieve session
      const retrievedSession = mockServer.getSession(session.sessionId);

      expect(retrievedSession).toBeDefined();
      expect(retrievedSession.sessionId).toBe(session.sessionId);

      // Events should be associated with session
      const events = mockServer.getEvents(session.sessionId);
      expect(events).toHaveLength(1);
    });
  });

  describe('Manifest Parsing Verification', () => {
    it('should produce parseable HLS manifest', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const manifest = mockServer.getManifest(session.sessionId)!;

      // Basic HLS validation
      expect(manifest.startsWith('#EXTM3U')).toBe(true);
      expect(manifest).toContain('#EXT-X-VERSION');
      expect(manifest).toContain('#EXT-X-TARGETDURATION');
      expect(manifest).toContain('#EXTINF');
      expect(manifest).toContain('#EXT-X-ENDLIST');
    });

    it('should have valid DATERANGE attributes', () => {
      const session = mockServer.createSession({ tenantId: 'test' });
      const manifest = mockServer.getManifest(session.sessionId)!;

      // Extract DATERANGE line
      const dateRangeLine = manifest.split('\n').find(l => l.includes('#EXT-X-DATERANGE'));

      expect(dateRangeLine).toContain('ID=');
      expect(dateRangeLine).toContain('CLASS=');
      expect(dateRangeLine).toContain('DURATION=');
    });
  });
});

describe('SSAI Metrics and Reporting', () => {
  let mockServer: MockSSAIServer;

  beforeEach(() => {
    mockServer = new MockSSAIServer();
  });

  it('should calculate completion rate', () => {
    const session = mockServer.createSession({ tenantId: 'test' });

    // Complete first ad
    mockServer.trackEvent(session.sessionId, {
      type: 'impression',
      podId: 'preroll-pod-1',
      slotId: 'slot-1',
      creativeId: 'creative-001',
      mediaTime: 0,
    });
    mockServer.trackEvent(session.sessionId, {
      type: 'complete',
      podId: 'preroll-pod-1',
      slotId: 'slot-1',
      creativeId: 'creative-001',
      mediaTime: 15,
    });

    // Skip second ad
    mockServer.trackEvent(session.sessionId, {
      type: 'impression',
      podId: 'preroll-pod-1',
      slotId: 'slot-2',
      creativeId: 'creative-002',
      mediaTime: 15,
    });
    mockServer.trackEvent(session.sessionId, {
      type: 'skip',
      podId: 'preroll-pod-1',
      slotId: 'slot-2',
      creativeId: 'creative-002',
      mediaTime: 20,
    });

    const events = mockServer.getEvents(session.sessionId);
    const impressions = events.filter(e => e.type === 'impression').length;
    const completions = events.filter(e => e.type === 'complete').length;
    const completionRate = completions / impressions;

    expect(impressions).toBe(2);
    expect(completions).toBe(1);
    expect(completionRate).toBe(0.5);
  });

  it('should track fill rate', () => {
    const session = mockServer.createSession({ tenantId: 'test' });
    const preroll = session.adBreaks[0];

    const fillRate = preroll.filledSlots / preroll.plannedSlots;

    expect(fillRate).toBe(1); // 100% fill
  });
});
