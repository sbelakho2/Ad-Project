/**
 * SSAI Stitching Module Unit Tests
 */

import { HLSStitcher } from '../stitching/HLSStitcher';
import { DASHStitcher } from '../stitching/DASHStitcher';
import { SegmentAligner } from '../stitching/SegmentAligner';
import { DRMPolicyHandler } from '../stitching/DRMPolicyHandler';
import { AudioLoudnessDetector } from '../stitching/AudioLoudnessDetector';
import { CaptionHandler } from '../stitching/CaptionHandler';

describe('HLSStitcher', () => {
  let stitcher: HLSStitcher;

  beforeEach(() => {
    stitcher = new HLSStitcher();
  });

  describe('stitch', () => {
    it('should stitch content and ad segments', () => {
      const request = {
        sessionId: 'sess-1',
        podId: 'pod-1',
        mode: 'replacement' as const,
        protocol: 'hls' as const,
        container: 'ts' as const,
        contentSegments: [
          { uri: 'content1.ts', duration: 6, startTime: 0 },
          { uri: 'content2.ts', duration: 6, startTime: 6 },
        ],
        adSegments: [
          { uri: 'ad1.ts', duration: 15, startTime: 0 },
          { uri: 'ad2.ts', duration: 15, startTime: 15 },
        ],
        breakPosition: 12,
        breakDuration: 30,
        isLive: false,
      };

      const result = stitcher.stitch(request);

      expect(result.segments).toBeDefined();
    });
  });

  describe('constructor', () => {
    it('should create with default options', () => {
      expect(stitcher).toBeDefined();
    });

    it('should create with custom options', () => {
      const customStitcher = new HLSStitcher({
        targetDuration: 10,
        slidingWindowSize: 5,
      });
      expect(customStitcher).toBeDefined();
    });
  });
});

describe('DASHStitcher', () => {
  let stitcher: DASHStitcher;

  beforeEach(() => {
    stitcher = new DASHStitcher();
  });

  describe('stitch', () => {
    it('should stitch content and ad segments', () => {
      const request = {
        sessionId: 'sess-1',
        podId: 'pod-1',
        mode: 'replacement' as const,
        protocol: 'dash' as const,
        container: 'cmaf' as const,
        contentSegments: [{ uri: 'content.m4s', duration: 60, startTime: 0 }],
        adSegments: [{ uri: 'ad.m4s', duration: 30, startTime: 0 }],
        breakPosition: 60,
        breakDuration: 30,
        isLive: false,
      };

      const result = stitcher.stitch(request);

      expect(result).toBeDefined();
    });
  });

  describe('constructor', () => {
    it('should create with default options', () => {
      expect(stitcher).toBeDefined();
    });
  });
});

describe('SegmentAligner', () => {
  let aligner: SegmentAligner;

  beforeEach(() => {
    aligner = new SegmentAligner({
      targetDuration: 6,
      maxVariance: 0.5,
    });
  });

  describe('alignSegments', () => {
    it('should pass aligned segments', () => {
      const segments = [
        { uri: 'seg1.ts', duration: 6, startTime: 0, endTime: 6, isKeyframe: true, hasAudio: true, hasVideo: true },
        { uri: 'seg2.ts', duration: 6, startTime: 6, endTime: 12, isKeyframe: true, hasAudio: true, hasVideo: true },
      ];

      const result = aligner.alignSegments(segments);

      expect(result.aligned).toBe(true);
      expect(result.adjustments.length).toBe(0);
    });

    it('should flag misaligned segments', () => {
      const segments = [
        { uri: 'seg1.ts', duration: 8, startTime: 0, endTime: 8, isKeyframe: true, hasAudio: true, hasVideo: true },
      ];

      const result = aligner.alignSegments(segments);

      expect(result.adjustments.length).toBeGreaterThan(0);
    });

    it('should warn on missing keyframe', () => {
      const segments = [
        { uri: 'seg1.ts', duration: 6, startTime: 0, endTime: 6, isKeyframe: true, hasAudio: true, hasVideo: true },
        { uri: 'seg2.ts', duration: 6, startTime: 6, endTime: 12, isKeyframe: false, hasAudio: true, hasVideo: true },
      ];

      const result = aligner.alignSegments(segments);

      expect(result.warnings.length).toBeGreaterThan(0);
    });
  });

  describe('findSplicePoint', () => {
    it('should find nearest GOP boundary', () => {
      const segments = [
        { uri: 'seg1.ts', duration: 6, startTime: 0, endTime: 6, isKeyframe: true, hasAudio: true, hasVideo: true },
        { uri: 'seg2.ts', duration: 6, startTime: 6, endTime: 12, isKeyframe: true, hasAudio: true, hasVideo: true },
      ];

      const splicePoint = aligner.findSplicePoint(segments, 5);

      expect(splicePoint).toBeDefined();
      expect(splicePoint?.isKeyframe).toBe(true);
    });
  });
});

describe('DRMPolicyHandler', () => {
  let handler: DRMPolicyHandler;

  beforeEach(() => {
    handler = new DRMPolicyHandler();
  });

  describe('validateCompatibility', () => {
    it('should allow clear content with clear ads', () => {
      const contentDrm = { scheme: 'none' as const, isEncrypted: false };
      const adDrm = { scheme: 'none' as const, isEncrypted: false };

      const result = handler.validateCompatibility(contentDrm, adDrm);

      expect(result.valid).toBe(true);
      expect(result.compatible).toBe(true);
    });

    it('should allow clear ads in encrypted content by default', () => {
      const contentDrm = { scheme: 'widevine' as const, isEncrypted: true, keyId: 'key-1' };
      const adDrm = { scheme: 'none' as const, isEncrypted: false };

      const result = handler.validateCompatibility(contentDrm, adDrm);

      expect(result.compatible).toBe(true);
      expect(result.warnings.length).toBeGreaterThan(0);
    });

    it('should reject encrypted ads in clear content', () => {
      const contentDrm = { scheme: 'none' as const, isEncrypted: false };
      const adDrm = { scheme: 'widevine' as const, isEncrypted: true, keyId: 'key-1' };

      const result = handler.validateCompatibility(contentDrm, adDrm);

      expect(result.compatible).toBe(false);
    });
  });

  describe('checkKeyContinuity', () => {
    it('should pass for same key', () => {
      const contentDrm = { scheme: 'widevine' as const, isEncrypted: true, keyId: 'key-1' };
      const adDrm = { scheme: 'widevine' as const, isEncrypted: true, keyId: 'key-1' };

      const result = handler.checkKeyContinuity(contentDrm, adDrm);

      expect(result.continuous).toBe(true);
    });
  });
});

describe('AudioLoudnessDetector', () => {
  let detector: AudioLoudnessDetector;

  beforeEach(() => {
    detector = new AudioLoudnessDetector({
      targetLufs: -24,
      maxDeviation: 2,
    });
  });

  describe('analyzeLoudness', () => {
    it('should pass compliant loudness', () => {
      const loudness = { integratedLufs: -24, loudnessRange: 10, truePeak: -2 };

      const result = detector.analyzeLoudness(loudness, 'content');

      expect(result.compliant).toBe(true);
    });

    it('should flag non-compliant loudness', () => {
      const loudness = { integratedLufs: -18, loudnessRange: 10, truePeak: 0 };

      const result = detector.analyzeLoudness(loudness, 'ad');

      expect(result.compliant).toBe(false);
    });
  });

  describe('compareLoudness', () => {
    it('should detect jarring difference', () => {
      const contentLoudness = { integratedLufs: -24, loudnessRange: 8, truePeak: -2 };
      const adLoudness = { integratedLufs: -16, loudnessRange: 6, truePeak: -1 };

      const result = detector.compareLoudness(contentLoudness, adLoudness);

      expect(result.jarring).toBe(true);
      expect(result.recommendation).toBeDefined();
    });
  });
});

describe('CaptionHandler', () => {
  let handler: CaptionHandler;

  beforeEach(() => {
    handler = new CaptionHandler();
  });

  describe('processContentCaptions', () => {
    it('should process valid captions', () => {
      const cues = [
        { id: 'cue-1', startTime: 0, endTime: 3, text: 'Hello world' },
        { id: 'cue-2', startTime: 3, endTime: 6, text: 'Goodbye world' },
      ];

      const result = handler.processContentCaptions(cues);

      expect(result.outputCues.length).toBe(2);
      expect(result.errors.length).toBe(0);
    });

    it('should validate cue timing', () => {
      const cues = [
        { id: 'cue-1', startTime: 5, endTime: 3, text: 'Invalid timing' },
      ];

      const result = handler.processContentCaptions(cues);

      expect(result.errors.length).toBeGreaterThan(0);
    });
  });

  describe('enterAdBreak', () => {
    it('should track ad break state', () => {
      handler.enterAdBreak(300);

      const state = handler.getState();

      expect(state.inAdBreak).toBe(true);
    });
  });

  describe('exitAdBreak', () => {
    it('should resume captions', () => {
      handler.enterAdBreak(300);
      const result = handler.exitAdBreak(330);

      expect(result.state.inAdBreak).toBe(false);
    });
  });
});
