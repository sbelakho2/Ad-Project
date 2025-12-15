/**
 * SSAI DASH Manifest Generator Unit Tests
 */

import { DASHManifestGenerator } from '../manifest/DASHManifestGenerator';

describe('DASHManifestGenerator', () => {
  let generator: DASHManifestGenerator;

  beforeEach(() => {
    generator = new DASHManifestGenerator();
  });

  describe('generateMPD', () => {
    it('should generate valid MPD with required elements', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'live' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
        segmentBaseUrl: 'https://cdn.example.com/',
      };

      const periods = [
        {
          id: 'period-1',
          start: 'PT0S',
          duration: 'PT30S',
          adaptationSets: [],
        },
      ];

      const mpd = generator.generateMPD(context, periods);

      expect(mpd).toContain('<?xml');
      expect(mpd).toContain('<MPD');
      expect(mpd).toContain('minBufferTime=');
      expect(mpd).toContain('type="dynamic"');
      expect(mpd).toContain('<Period');
    });

    it('should include static type for VOD', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
        segmentBaseUrl: 'https://cdn.example.com/',
      };

      const periods = [
        { id: 'period-1', start: 'PT0S', adaptationSets: [] },
      ];

      const mpd = generator.generateMPD(context, periods, 'PT3600S');

      expect(mpd).toContain('type="static"');
      expect(mpd).toContain('mediaPresentationDuration="PT3600S"');
    });

    it('should generate multi-period MPD', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
        segmentBaseUrl: 'https://cdn.example.com/',
      };

      const periods = [
        { id: 'content-1', start: 'PT0S', duration: 'PT300S', adaptationSets: [] },
        { id: 'ad-1', start: 'PT300S', duration: 'PT30S', adaptationSets: [] },
        { id: 'content-2', start: 'PT330S', duration: 'PT300S', adaptationSets: [] },
      ];

      const mpd = generator.generateMPD(context, periods);

      expect(mpd).toContain('id="content-1"');
      expect(mpd).toContain('id="ad-1"');
      expect(mpd).toContain('id="content-2"');
    });
  });

  describe('UTCTiming', () => {
    it('should include UTCTiming for live', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'live' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
        segmentBaseUrl: 'https://cdn.example.com/',
      };

      const periods = [{ id: 'period-1', start: 'PT0S', adaptationSets: [] }];

      const mpd = generator.generateMPD(context, periods);

      expect(mpd).toContain('<UTCTiming');
    });

    it('should not include UTCTiming for VOD', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
        segmentBaseUrl: 'https://cdn.example.com/',
      };

      const periods = [{ id: 'period-1', start: 'PT0S', adaptationSets: [] }];

      const mpd = generator.generateMPD(context, periods);

      expect(mpd).not.toContain('<UTCTiming');
    });
  });

  describe('constructor', () => {
    it('should create with default options', () => {
      const gen = new DASHManifestGenerator();
      expect(gen).toBeDefined();
    });

    it('should create with custom options', () => {
      const gen = new DASHManifestGenerator({
        minBufferTime: 'PT4S',
        minUpdatePeriod: 'PT4S',
      });
      expect(gen).toBeDefined();
    });
  });
});
