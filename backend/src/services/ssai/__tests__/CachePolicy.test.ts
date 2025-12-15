/**
 * SSAI Cache Policy Unit Tests
 */

import { CachePolicy } from '../manifest/CachePolicy';

describe('CachePolicy', () => {
  let policy: CachePolicy;

  beforeEach(() => {
    policy = new CachePolicy();
  });

  describe('getHeaders', () => {
    it('should return no-store for session manifests', () => {
      const headers = policy.getHeaders('session-manifest');

      expect(headers['Cache-Control']).toContain('no-store');
    });

    it('should return short TTL for live manifests', () => {
      const headers = policy.getHeaders('live-manifest');

      expect(headers['Cache-Control']).toMatch(/max-age=\d+/);
    });

    it('should return longer TTL for VOD manifests', () => {
      const headers = policy.getHeaders('vod-manifest');

      expect(headers['Cache-Control']).toMatch(/max-age=\d+/);
    });

    it('should return long TTL for segments', () => {
      const headers = policy.getHeaders('segment');

      expect(headers['Cache-Control']).toMatch(/max-age=\d+/);
    });

    it('should return long TTL for init segments', () => {
      const headers = policy.getHeaders('init-segment');

      expect(headers['Cache-Control']).toMatch(/max-age=\d+/);
    });
  });

  describe('constructor', () => {
    it('should create with default config', () => {
      expect(policy).toBeDefined();
    });

    it('should create with custom config', () => {
      const customPolicy = new CachePolicy({
        liveManifestTtl: 5,
        vodManifestTtl: 600,
      });
      expect(customPolicy).toBeDefined();
    });
  });
});
