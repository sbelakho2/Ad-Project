/**
 * SSAI Container Handler Unit Tests
 */

import { ContainerHandler } from '../manifest/ContainerHandler';

describe('ContainerHandler', () => {
  let handler: ContainerHandler;

  beforeEach(() => {
    handler = new ContainerHandler();
  });

  describe('detectContainerType', () => {
    it('should detect TS container from URI', () => {
      const type = handler.detectContainerType('segment.ts');

      expect(type).toBe('ts');
    });

    it('should detect CMAF container from URI', () => {
      const type = handler.detectContainerType('segment.m4s');

      expect(type).toBe('cmaf');
    });

    it('should detect fMP4 init segment as CMAF', () => {
      const type = handler.detectContainerType('init.mp4');

      expect(type).toBe('cmaf');
    });

    it('should return unknown for unrecognized extension', () => {
      const type = handler.detectContainerType('segment.xyz');

      expect(type).toBe('unknown');
    });
  });

  describe('getContainerInfo', () => {
    it('should return TS container info', () => {
      const info = handler.getContainerInfo('segment.ts');

      expect(info.type).toBe('ts');
      expect(info.extension).toBe('.ts');
      expect(info.initSegmentRequired).toBe(false);
    });

    it('should return CMAF container info', () => {
      const info = handler.getContainerInfo('segment.m4s');

      expect(info.type).toBe('cmaf');
      expect(info.extension).toBe('.m4s');
      expect(info.initSegmentRequired).toBe(true);
    });
  });

  describe('getMimeType', () => {
    it('should return correct MIME type for TS video', () => {
      const container = { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' };
      const mimeType = handler.getMimeType(container, 'video');

      expect(mimeType).toBe('video/MP2T');
    });

    it('should return correct MIME type for CMAF video', () => {
      const container = { type: 'cmaf' as const, extension: '.m4s', mimeType: 'video/mp4' };
      const mimeType = handler.getMimeType(container, 'video');

      expect(mimeType).toBe('video/mp4');
    });

    it('should return correct MIME type for CMAF audio', () => {
      const container = { type: 'cmaf' as const, extension: '.m4s', mimeType: 'audio/mp4' };
      const mimeType = handler.getMimeType(container, 'audio');

      expect(mimeType).toBe('audio/mp4');
    });
  });

  describe('checkCompatibility', () => {
    it('should allow same container types', () => {
      const content = { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' };
      const ad = { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' };

      const result = handler.checkCompatibility(content, ad);

      expect(result.compatible).toBe(true);
    });

    it('should reject mismatched container types', () => {
      const content = { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' };
      const ad = { type: 'cmaf' as const, extension: '.m4s', mimeType: 'video/mp4' };

      const result = handler.checkCompatibility(content, ad);

      expect(result.compatible).toBe(false);
      expect(result.reason).toBeDefined();
    });
  });

  describe('validateSegment', () => {
    it('should validate segment has required properties', () => {
      const segment = {
        uri: 'segment.ts',
        container: { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' },
        startTime: 0,
        duration: 6,
        discontinuity: false,
        isAd: false,
      };

      const result = handler.validateSegment(segment);

      expect(result.valid).toBe(true);
    });

    it('should reject segment with zero duration', () => {
      const segment = {
        uri: 'segment.ts',
        container: { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' },
        startTime: 0,
        duration: 0,
        discontinuity: false,
        isAd: false,
      };

      const result = handler.validateSegment(segment);

      expect(result.valid).toBe(false);
    });

    it('should reject segment without URI', () => {
      const segment = {
        uri: '',
        container: { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' },
        startTime: 0,
        duration: 6,
        discontinuity: false,
        isAd: false,
      };

      const result = handler.validateSegment(segment);

      expect(result.valid).toBe(false);
    });

    it('should reject ad segment without creativeId', () => {
      const segment = {
        uri: 'segment.ts',
        container: { type: 'ts' as const, extension: '.ts', mimeType: 'video/MP2T' },
        startTime: 0,
        duration: 6,
        discontinuity: false,
        isAd: true,
      };

      const result = handler.validateSegment(segment);

      expect(result.valid).toBe(false);
    });
  });
});
