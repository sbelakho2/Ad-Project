/**
 * SSAI HLS Manifest Generator Unit Tests
 */

import { HLSManifestGenerator } from '../manifest/HLSManifestGenerator';

describe('HLSManifestGenerator', () => {
  let generator: HLSManifestGenerator;

  beforeEach(() => {
    generator = new HLSManifestGenerator();
  });

  describe('generateMasterPlaylist', () => {
    it('should generate valid master playlist with #EXTM3U header', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
      };

      const renditions = [
        { bandwidth: 2000000, codecs: 'avc1.4d401f,mp4a.40.2', width: 1280, height: 720, uri: 'variant1.m3u8' },
        { bandwidth: 5000000, codecs: 'avc1.640028,mp4a.40.2', width: 1920, height: 1080, uri: 'variant2.m3u8' },
      ];

      const playlist = generator.generateMasterPlaylist(context, renditions);

      expect(playlist).toContain('#EXTM3U');
      expect(playlist).toContain('#EXT-X-VERSION');
      expect(playlist).toContain('#EXT-X-STREAM-INF');
      expect(playlist).toContain('BANDWIDTH=2000000');
      expect(playlist).toContain('RESOLUTION=1280x720');
    });

    it('should include all variants', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
      };

      const renditions = [
        { bandwidth: 1000000, codecs: 'avc1.4d401f,mp4a.40.2', width: 640, height: 360, uri: 'low.m3u8' },
        { bandwidth: 2000000, codecs: 'avc1.4d401f,mp4a.40.2', width: 1280, height: 720, uri: 'mid.m3u8' },
        { bandwidth: 5000000, codecs: 'avc1.640028,mp4a.40.2', width: 1920, height: 1080, uri: 'high.m3u8' },
      ];

      const playlist = generator.generateMasterPlaylist(context, renditions);

      expect(playlist).toContain('low.m3u8');
      expect(playlist).toContain('mid.m3u8');
      expect(playlist).toContain('high.m3u8');
    });
  });

  describe('generateMediaPlaylist', () => {
    it('should generate valid media playlist with required tags', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
      };

      const segments = [
        { uri: 'seg1.ts', duration: 6 },
        { uri: 'seg2.ts', duration: 6 },
        { uri: 'seg3.ts', duration: 6 },
      ];

      const playlist = generator.generateMediaPlaylist(context, segments);

      expect(playlist).toContain('#EXTM3U');
      expect(playlist).toContain('#EXT-X-TARGETDURATION:6');
      expect(playlist).toContain('#EXT-X-MEDIA-SEQUENCE:0');
      expect(playlist).toContain('#EXTINF:');
      expect(playlist).toContain('seg1.ts');
    });

    it('should include EXT-X-ENDLIST for VOD', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 6,
        mediaSequence: 0,
        discontinuitySequence: 0,
      };

      const segments = [{ uri: 'seg1.ts', duration: 6 }];

      const playlist = generator.generateMediaPlaylist(context, segments);

      expect(playlist).toContain('#EXT-X-ENDLIST');
    });

    it('should not include EXT-X-PLAYLIST-TYPE for live', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'live' as const,
        targetDuration: 6,
        mediaSequence: 100,
        discontinuitySequence: 0,
      };

      const segments = [{ uri: 'seg1.ts', duration: 6 }];

      const playlist = generator.generateMediaPlaylist(context, segments);

      expect(playlist).not.toContain('#EXT-X-PLAYLIST-TYPE');
      expect(playlist).not.toContain('#EXT-X-ENDLIST');
    });

    it('should include EXT-X-DISCONTINUITY when specified', () => {
      const context = {
        sessionId: 'sess-1',
        token: 'token-1',
        mode: 'vod' as const,
        targetDuration: 15,
        mediaSequence: 0,
        discontinuitySequence: 0,
      };

      const segments = [
        { uri: 'content1.ts', duration: 6 },
        { uri: 'ad1.ts', duration: 15, discontinuity: true },
        { uri: 'content2.ts', duration: 6, discontinuity: true },
      ];

      const playlist = generator.generateMediaPlaylist(context, segments);

      expect(playlist).toContain('#EXT-X-DISCONTINUITY');
    });
  });

  describe('constructor', () => {
    it('should create with default options', () => {
      const gen = new HLSManifestGenerator();
      expect(gen).toBeDefined();
    });

    it('should create with custom options', () => {
      const gen = new HLSManifestGenerator({
        includeProgramDateTime: false,
        includeDateRange: false,
        liveWindowSize: 3,
      });
      expect(gen).toBeDefined();
    });
  });
});
