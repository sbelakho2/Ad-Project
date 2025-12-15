/**
 * Piercing E2E Tests - Stitching & Packaging
 *
 * Tests: "ABR boundary torture + drift + discontinuity correctness"
 * - ABR boundary torture test
 * - A/V drift soak test
 * - Sliding-window discontinuity edge test
 * - TS vs CMAF correctness test
 */

import { describe, it, expect, beforeEach, jest } from '@jest/globals';

interface Segment {
  uri: string;
  duration: number;
  startTime: number;
  endTime: number;
  discontinuity?: boolean;
  container: 'ts' | 'cmaf';
  isAd: boolean;
  bitrate: number;
  initSegment?: string;
}

interface DiscontinuitySequence {
  sequenceNumber: number;
  position: number;
  reason: 'ad_start' | 'ad_end' | 'origin_restart' | 'codec_change';
}

interface AVDriftMeasurement {
  timestamp: number;
  audioPts: number;
  videoPts: number;
  driftMs: number;
}

interface StitchingResult {
  segments: Segment[];
  discontinuities: DiscontinuitySequence[];
  valid: boolean;
  errors: string[];
  warnings: string[];
}

/**
 * Mock Stitching Service
 */
class MockStitchingService {
  private config = {
    targetDuration: 6,
    maxAvDriftMs: 40,
    maxDiscontinuityDrift: 100,
    liveWindowDuration: 300, // 5 minutes
  };

  /**
   * Stitch content with ads for HLS
   */
  stitchHLS(
    contentSegments: Segment[],
    adSegments: Segment[],
    adInsertPoint: number
  ): StitchingResult {
    const segments: Segment[] = [];
    const discontinuities: DiscontinuitySequence[] = [];
    const errors: string[] = [];
    const warnings: string[] = [];

    let currentTime = 0;
    let discoSeq = 0;

    // Add content before ad
    for (const seg of contentSegments) {
      if (seg.startTime < adInsertPoint) {
        segments.push({
          ...seg,
          startTime: currentTime,
          endTime: currentTime + seg.duration,
        });
        currentTime += seg.duration;
      }
    }

    // Add discontinuity for ad start
    discontinuities.push({
      sequenceNumber: discoSeq++,
      position: currentTime,
      reason: 'ad_start',
    });

    // Add ad segments
    for (const ad of adSegments) {
      segments.push({
        ...ad,
        discontinuity: segments.length === 0 ? false : true,
        startTime: currentTime,
        endTime: currentTime + ad.duration,
      });
      currentTime += ad.duration;
    }

    // Add discontinuity for ad end
    discontinuities.push({
      sequenceNumber: discoSeq++,
      position: currentTime,
      reason: 'ad_end',
    });

    // Add content after ad
    for (const seg of contentSegments) {
      if (seg.startTime >= adInsertPoint) {
        segments.push({
          ...seg,
          discontinuity: true,
          startTime: currentTime,
          endTime: currentTime + seg.duration,
        });
        currentTime += seg.duration;
      }
    }

    // Validate stitching
    const validation = this.validateStitching(segments, discontinuities);

    return {
      segments,
      discontinuities,
      valid: validation.valid,
      errors: validation.errors,
      warnings: validation.warnings,
    };
  }

  /**
   * Validate ABR switch at boundary
   */
  validateAbrSwitchAtBoundary(
    beforeSwitch: Segment,
    afterSwitch: Segment,
    isBoundary: boolean
  ): { valid: boolean; errors: string[] } {
    const errors: string[] = [];

    // Check container compatibility
    if (beforeSwitch.container !== afterSwitch.container) {
      errors.push(
        `Container mismatch at ABR switch: ${beforeSwitch.container} -> ${afterSwitch.container}`
      );
    }

    // Check for init segment when needed
    if (afterSwitch.container === 'cmaf' && !afterSwitch.initSegment) {
      errors.push('Missing init segment after CMAF ABR switch');
    }

    // Check timing continuity
    const timingGap = Math.abs(afterSwitch.startTime - beforeSwitch.endTime);
    if (timingGap > 0.1) {
      // 100ms tolerance
      errors.push(`Timing gap at ABR switch: ${(timingGap * 1000).toFixed(0)}ms`);
    }

    return {
      valid: errors.length === 0,
      errors,
    };
  }

  /**
   * Measure A/V drift
   */
  measureAvDrift(
    audioPts: number,
    videoPts: number,
    timestamp: number
  ): AVDriftMeasurement {
    // Convert PTS difference to milliseconds (90kHz clock)
    const driftMs = Math.abs(audioPts - videoPts) / 90;

    return {
      timestamp,
      audioPts,
      videoPts,
      driftMs,
    };
  }

  /**
   * Check if drift is within tolerance
   */
  isDriftAcceptable(measurement: AVDriftMeasurement): boolean {
    return measurement.driftMs <= this.config.maxAvDriftMs;
  }

  /**
   * Validate sliding window discontinuity coherence
   */
  validateSlidingWindowDiscontinuities(
    previousWindow: DiscontinuitySequence[],
    currentWindow: DiscontinuitySequence[]
  ): { coherent: boolean; errors: string[] } {
    const errors: string[] = [];

    // Find overlapping sequences to verify coherence
    if (currentWindow.length > 0 && previousWindow.length > 0) {
      // Check for gaps or regressions in the overlap
      const prevSeqNumbers = new Set(previousWindow.map((d) => d.sequenceNumber));
      const currentSeqNumbers = new Set(currentWindow.map((d) => d.sequenceNumber));

      // At least one sequence should overlap between windows for coherence
      const overlap = [...prevSeqNumbers].filter((s) => currentSeqNumbers.has(s));
      if (overlap.length === 0) {
        // No overlap - check if there's a gap in sequences
        const maxPrev = Math.max(...previousWindow.map((d) => d.sequenceNumber));
        const minCurrent = Math.min(...currentWindow.map((d) => d.sequenceNumber));
        const maxCurrent = Math.max(...currentWindow.map((d) => d.sequenceNumber));
        
        // Regression: current window ends before previous window max
        if (maxCurrent < maxPrev) {
          errors.push(
            `Discontinuity sequence regression: max was ${maxPrev}, current max is ${maxCurrent}`
          );
        }
        // Gap in sequences
        if (minCurrent > maxPrev + 1) {
          errors.push(
            `Discontinuity sequence gap: ${maxPrev} -> ${minCurrent}`
          );
        }
      }
    }

    // Check for duplicate sequence numbers within current window
    const seqNumbers = currentWindow.map((d) => d.sequenceNumber);
    const uniqueSeqNumbers = new Set(seqNumbers);
    if (seqNumbers.length !== uniqueSeqNumbers.size) {
      errors.push('Duplicate discontinuity sequence numbers detected');
    }

    return {
      coherent: errors.length === 0,
      errors,
    };
  }

  /**
   * Validate container format correctness
   */
  validateContainerFormat(
    segment: Segment,
    expectedContainer: 'ts' | 'cmaf'
  ): { valid: boolean; error?: string } {
    if (segment.container !== expectedContainer) {
      return {
        valid: false,
        error: `Wrong container format: expected ${expectedContainer}, got ${segment.container}`,
      };
    }

    // Additional CMAF validations
    if (expectedContainer === 'cmaf') {
      if (!segment.uri.endsWith('.m4s') && !segment.uri.endsWith('.mp4')) {
        return {
          valid: false,
          error: `CMAF segment should have .m4s or .mp4 extension: ${segment.uri}`,
        };
      }
    }

    // Additional TS validations
    if (expectedContainer === 'ts') {
      if (!segment.uri.endsWith('.ts')) {
        return {
          valid: false,
          error: `TS segment should have .ts extension: ${segment.uri}`,
        };
      }
    }

    return { valid: true };
  }

  private validateStitching(
    segments: Segment[],
    discontinuities: DiscontinuitySequence[]
  ): { valid: boolean; errors: string[]; warnings: string[] } {
    const errors: string[] = [];
    const warnings: string[] = [];

    // Check segment continuity
    for (let i = 1; i < segments.length; i++) {
      const gap = segments[i].startTime - segments[i - 1].endTime;
      if (Math.abs(gap) > 0.1) {
        if (segments[i].discontinuity) {
          // Gap is expected at discontinuity
        } else {
          errors.push(`Unexpected gap at segment ${i}: ${gap}s`);
        }
      }
    }

    // Check discontinuity markers
    for (const disco of discontinuities) {
      const segmentAtPosition = segments.find(
        (s) => Math.abs(s.startTime - disco.position) < 0.1
      );
      if (!segmentAtPosition) {
        warnings.push(
          `Discontinuity at ${disco.position}s has no matching segment`
        );
      }
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
    };
  }
}

describe('Piercing E2E Tests - Stitching & Packaging', () => {
  let stitchingService: MockStitchingService;

  beforeEach(() => {
    stitchingService = new MockStitchingService();
  });

  describe('ABR Boundary Torture Test', () => {
    /**
     * Force ABR switches precisely at ad start and ad end
     * → no fatal, no infinite buffering
     */
    it('should handle ABR switch at ad start boundary', () => {
      const beforeSwitch: Segment = {
        uri: 'content-1000.ts',
        duration: 6,
        startTime: 100,
        endTime: 106,
        container: 'ts',
        isAd: false,
        bitrate: 5000000,
      };

      const afterSwitch: Segment = {
        uri: 'ad-001.ts',
        duration: 6,
        startTime: 106,
        endTime: 112,
        container: 'ts',
        isAd: true,
        bitrate: 3000000, // Different bitrate
        discontinuity: true,
      };

      const result = stitchingService.validateAbrSwitchAtBoundary(
        beforeSwitch,
        afterSwitch,
        true // At ad start boundary
      );

      expect(result.valid).toBe(true);
      expect(result.errors.length).toBe(0);
    });

    it('should handle ABR switch at ad end boundary', () => {
      const beforeSwitch: Segment = {
        uri: 'ad-005.ts',
        duration: 6,
        startTime: 130,
        endTime: 136,
        container: 'ts',
        isAd: true,
        bitrate: 3000000,
      };

      const afterSwitch: Segment = {
        uri: 'content-1005.ts',
        duration: 6,
        startTime: 136,
        endTime: 142,
        container: 'ts',
        isAd: false,
        bitrate: 5000000,
        discontinuity: true,
      };

      const result = stitchingService.validateAbrSwitchAtBoundary(
        beforeSwitch,
        afterSwitch,
        true // At ad end boundary
      );

      expect(result.valid).toBe(true);
      expect(result.errors.length).toBe(0);
    });

    it('should detect container mismatch at ABR switch', () => {
      const beforeSwitch: Segment = {
        uri: 'content-1000.ts',
        duration: 6,
        startTime: 100,
        endTime: 106,
        container: 'ts',
        isAd: false,
        bitrate: 5000000,
      };

      const afterSwitch: Segment = {
        uri: 'ad-001.m4s',
        duration: 6,
        startTime: 106,
        endTime: 112,
        container: 'cmaf', // Different container!
        isAd: true,
        bitrate: 3000000,
      };

      const result = stitchingService.validateAbrSwitchAtBoundary(
        beforeSwitch,
        afterSwitch,
        true
      );

      expect(result.valid).toBe(false);
      expect(result.errors).toContain('Container mismatch at ABR switch: ts -> cmaf');
    });

    it('should require init segment for CMAF ABR switch', () => {
      const beforeSwitch: Segment = {
        uri: 'content-1000.m4s',
        duration: 6,
        startTime: 100,
        endTime: 106,
        container: 'cmaf',
        isAd: false,
        bitrate: 5000000,
        initSegment: 'init-high.mp4',
      };

      const afterSwitch: Segment = {
        uri: 'ad-001.m4s',
        duration: 6,
        startTime: 106,
        endTime: 112,
        container: 'cmaf',
        isAd: true,
        bitrate: 3000000,
        // Missing init segment!
      };

      const result = stitchingService.validateAbrSwitchAtBoundary(
        beforeSwitch,
        afterSwitch,
        true
      );

      expect(result.valid).toBe(false);
      expect(result.errors).toContain('Missing init segment after CMAF ABR switch');
    });
  });

  describe('A/V Drift Soak Test', () => {
    /**
     * 2-6 hour live run with frequent breaks
     * → assert drift stays below threshold; no gradual desync
     */
    it('should maintain A/V sync within tolerance', () => {
      const measurements: AVDriftMeasurement[] = [];

      // Simulate 2 hours of measurements (every 10 seconds)
      const durationSeconds = 2 * 60 * 60;
      const measurementInterval = 10;
      let basePts = 0;

      for (let t = 0; t < durationSeconds; t += measurementInterval) {
        // Simulate small random drift
        const randomDrift = (Math.random() - 0.5) * 20; // +/- 10ms
        const audioPts = basePts + t * 90000; // 90kHz PTS
        const videoPts = audioPts + randomDrift * 90; // Drift in PTS units

        const measurement = stitchingService.measureAvDrift(
          audioPts,
          videoPts,
          t * 1000
        );
        measurements.push(measurement);

        expect(stitchingService.isDriftAcceptable(measurement)).toBe(true);
      }

      // Check overall drift statistics
      const maxDrift = Math.max(...measurements.map((m) => m.driftMs));
      const avgDrift =
        measurements.reduce((sum, m) => sum + m.driftMs, 0) / measurements.length;

      expect(maxDrift).toBeLessThan(40); // 40ms max
      expect(avgDrift).toBeLessThan(20); // 20ms average
    });

    it('should detect gradual desync', () => {
      const measurements: AVDriftMeasurement[] = [];
      let cumulativeDrift = 0;

      // Simulate accumulating drift (bad encoder scenario)
      // Need to accumulate enough drift to exceed 40ms threshold
      for (let t = 0; t < 1000; t += 10) {
        cumulativeDrift += 1; // 1ms per measurement - will exceed 40ms at iteration 41
        const audioPts = t * 90000;
        const videoPts = audioPts + cumulativeDrift * 90; // Convert ms to PTS units

        const measurement = stitchingService.measureAvDrift(
          audioPts,
          videoPts,
          t * 1000
        );
        measurements.push(measurement);
      }

      // Eventually drift should exceed threshold
      const exceedsThreshold = measurements.some(
        (m) => !stitchingService.isDriftAcceptable(m)
      );
      expect(exceedsThreshold).toBe(true);

      // Calculate when threshold was exceeded
      const firstExceedance = measurements.findIndex(
        (m) => !stitchingService.isDriftAcceptable(m)
      );
      expect(firstExceedance).toBeGreaterThan(0);
    });
  });

  describe('Sliding-Window Discontinuity Edge Test', () => {
    /**
     * Breaks repeatedly occur at live window edge
     * → discontinuity sequence remains coherent; hls.js/dash.js don't accumulate warnings
     */
    it('should maintain coherent discontinuity sequence across window slides', () => {
      // Simulate sliding window updates
      const windowUpdates: DiscontinuitySequence[][] = [];

      // Window 1: sequences 0-2
      windowUpdates.push([
        { sequenceNumber: 0, position: 10, reason: 'ad_start' },
        { sequenceNumber: 1, position: 40, reason: 'ad_end' },
        { sequenceNumber: 2, position: 60, reason: 'ad_start' },
      ]);

      // Window 2: sequences 1-3 (window slides, 0 drops off)
      windowUpdates.push([
        { sequenceNumber: 1, position: 40, reason: 'ad_end' },
        { sequenceNumber: 2, position: 60, reason: 'ad_start' },
        { sequenceNumber: 3, position: 90, reason: 'ad_end' },
      ]);

      // Window 3: sequences 2-4
      windowUpdates.push([
        { sequenceNumber: 2, position: 60, reason: 'ad_start' },
        { sequenceNumber: 3, position: 90, reason: 'ad_end' },
        { sequenceNumber: 4, position: 120, reason: 'ad_start' },
      ]);

      // Validate coherence between consecutive windows
      for (let i = 1; i < windowUpdates.length; i++) {
        const validation = stitchingService.validateSlidingWindowDiscontinuities(
          windowUpdates[i - 1],
          windowUpdates[i]
        );
        expect(validation.coherent).toBe(true);
        expect(validation.errors.length).toBe(0);
      }
    });

    it('should detect discontinuity sequence regression', () => {
      const previousWindow: DiscontinuitySequence[] = [
        { sequenceNumber: 5, position: 100, reason: 'ad_start' },
        { sequenceNumber: 6, position: 130, reason: 'ad_end' },
      ];

      // Regression: current window starts with sequence lower than min of previous
      const currentWindow: DiscontinuitySequence[] = [
        { sequenceNumber: 2, position: 80, reason: 'ad_end' }, // Clear regression (gap > 1)
        { sequenceNumber: 3, position: 100, reason: 'ad_start' },
      ];

      const validation = stitchingService.validateSlidingWindowDiscontinuities(
        previousWindow,
        currentWindow
      );

      expect(validation.coherent).toBe(false);
      expect(validation.errors.length).toBeGreaterThan(0);
    });

    it('should detect duplicate discontinuity sequences', () => {
      const previousWindow: DiscontinuitySequence[] = [];

      const currentWindow: DiscontinuitySequence[] = [
        { sequenceNumber: 1, position: 10, reason: 'ad_start' },
        { sequenceNumber: 1, position: 40, reason: 'ad_end' }, // Duplicate!
        { sequenceNumber: 2, position: 60, reason: 'ad_start' },
      ];

      const validation = stitchingService.validateSlidingWindowDiscontinuities(
        previousWindow,
        currentWindow
      );

      expect(validation.coherent).toBe(false);
      expect(validation.errors).toContain(
        'Duplicate discontinuity sequence numbers detected'
      );
    });
  });

  describe('TS vs CMAF Correctness Test', () => {
    /**
     * Run same channel in TS/HLS and CMAF/HLS and both DASH variants
     * → wrong-format creatives must be rejected with explicit reason
     */
    it('should validate TS segment format', () => {
      const tsSegment: Segment = {
        uri: 'segment-001.ts',
        duration: 6,
        startTime: 0,
        endTime: 6,
        container: 'ts',
        isAd: false,
        bitrate: 5000000,
      };

      const result = stitchingService.validateContainerFormat(tsSegment, 'ts');
      expect(result.valid).toBe(true);
    });

    it('should reject CMAF segment in TS stream', () => {
      const cmafSegment: Segment = {
        uri: 'segment-001.m4s',
        duration: 6,
        startTime: 0,
        endTime: 6,
        container: 'cmaf',
        isAd: true,
        bitrate: 5000000,
      };

      const result = stitchingService.validateContainerFormat(cmafSegment, 'ts');
      expect(result.valid).toBe(false);
      expect(result.error).toContain('Wrong container format');
    });

    it('should validate CMAF segment format', () => {
      const cmafSegment: Segment = {
        uri: 'segment-001.m4s',
        duration: 6,
        startTime: 0,
        endTime: 6,
        container: 'cmaf',
        isAd: false,
        bitrate: 5000000,
        initSegment: 'init.mp4',
      };

      const result = stitchingService.validateContainerFormat(cmafSegment, 'cmaf');
      expect(result.valid).toBe(true);
    });

    it('should reject TS segment in CMAF stream', () => {
      const tsSegment: Segment = {
        uri: 'segment-001.ts',
        duration: 6,
        startTime: 0,
        endTime: 6,
        container: 'ts',
        isAd: true,
        bitrate: 5000000,
      };

      const result = stitchingService.validateContainerFormat(tsSegment, 'cmaf');
      expect(result.valid).toBe(false);
      expect(result.error).toContain('Wrong container format');
    });

    it('should validate CMAF file extension', () => {
      const badExtension: Segment = {
        uri: 'segment-001.xyz', // Wrong extension
        duration: 6,
        startTime: 0,
        endTime: 6,
        container: 'cmaf',
        isAd: false,
        bitrate: 5000000,
      };

      const result = stitchingService.validateContainerFormat(badExtension, 'cmaf');
      expect(result.valid).toBe(false);
      expect(result.error).toContain('.m4s or .mp4 extension');
    });
  });

  describe('Full Stitching Integration', () => {
    it('should stitch content with ads correctly', () => {
      const contentSegments: Segment[] = [
        { uri: 'content-001.ts', duration: 6, startTime: 0, endTime: 6, container: 'ts', isAd: false, bitrate: 5000000 },
        { uri: 'content-002.ts', duration: 6, startTime: 6, endTime: 12, container: 'ts', isAd: false, bitrate: 5000000 },
        { uri: 'content-003.ts', duration: 6, startTime: 12, endTime: 18, container: 'ts', isAd: false, bitrate: 5000000 },
        { uri: 'content-004.ts', duration: 6, startTime: 18, endTime: 24, container: 'ts', isAd: false, bitrate: 5000000 },
      ];

      const adSegments: Segment[] = [
        { uri: 'ad-001.ts', duration: 6, startTime: 0, endTime: 6, container: 'ts', isAd: true, bitrate: 3000000 },
        { uri: 'ad-002.ts', duration: 6, startTime: 6, endTime: 12, container: 'ts', isAd: true, bitrate: 3000000 },
      ];

      const result = stitchingService.stitchHLS(contentSegments, adSegments, 12);

      expect(result.valid).toBe(true);
      expect(result.segments.length).toBe(6); // 2 content + 2 ads + 2 content
      expect(result.discontinuities.length).toBe(2); // ad_start + ad_end
      expect(result.errors.length).toBe(0);
    });
  });
});

export {};
