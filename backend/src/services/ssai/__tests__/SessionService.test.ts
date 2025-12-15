/**
 * SSAI Session Service Unit Tests
 */

import { SessionService } from '../session/SessionService';
import { sessionStore } from '../session/SessionStore';

describe('SessionService', () => {
  let sessionService: SessionService;

  beforeEach(() => {
    sessionService = new SessionService(sessionStore);
  });

  afterEach(() => {
    jest.clearAllMocks();
  });

  describe('createSession', () => {
    it('should create a new session with valid config', async () => {
      const request = {
        config: {
          tenantId: 'tenant-1',
          channelId: 'channel-1',
          deviceProfile: {
            protocol: 'hls' as const,
            container: 'ts' as const,
          },
          isLive: true,
        },
      };

      const result = await sessionService.createSession(request);

      expect(result).toBeDefined();
      expect(result.created).toBe(true);
      expect(result.session).toBeDefined();
      expect(result.session.sessionId).toBeDefined();
      expect(result.manifestUrls).toBeDefined();
    });

    it('should return manifest URLs', async () => {
      const request = {
        config: {
          tenantId: 'tenant-1',
          channelId: 'channel-1',
          deviceProfile: {
            protocol: 'hls' as const,
            container: 'ts' as const,
          },
          isLive: true,
        },
      };

      const result = await sessionService.createSession(request);

      expect(result.manifestUrls.master).toBeDefined();
      expect(result.manifestUrls.master).toContain('m3u8');
    });
  });

  describe('getSession', () => {
    it('should return null for non-existent session', async () => {
      const session = await sessionService.getSession('non-existent-id');

      expect(session).toBeNull();
    });
  });

  describe('constructor', () => {
    it('should create with default store', () => {
      const service = new SessionService();
      expect(service).toBeDefined();
    });

    it('should create with custom store', () => {
      const service = new SessionService(sessionStore);
      expect(service).toBeDefined();
    });
  });
});
