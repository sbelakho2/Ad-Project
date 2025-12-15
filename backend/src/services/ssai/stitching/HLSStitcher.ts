/**
 * HLS Stitcher
 *
 * Handles HLS-specific stitching operations including
 * discontinuity management and sliding window handling.
 */

import {
  StitchingRequest,
  StitchingResult,
  StitchedSegment,
  ContentSegment,
  AdSegment,
} from './types';

export interface HLSStitchingOptions {
  /** Target duration for the playlist */
  targetDuration: number;

  /** Sliding window size (number of segments, 0 = VOD) */
  slidingWindowSize: number;

  /** Include DATERANGE tags */
  includeDateRange: boolean;

  /** Include PROGRAM-DATE-TIME */
  includeProgramDateTime: boolean;

  /** Start program date time */
  startProgramDateTime?: Date;
}

/**
 * HLS Stitcher for seamless ad insertion into HLS playlists.
 */
export class HLSStitcher {
  private options: HLSStitchingOptions;

  constructor(options?: Partial<HLSStitchingOptions>) {
    this.options = {
      targetDuration: 6,
      slidingWindowSize: 0,
      includeDateRange: true,
      includeProgramDateTime: true,
      ...options,
    };
  }

  /**
   * Stitch ads into an HLS playlist.
   */
  stitch(request: StitchingRequest): StitchingResult {
    const errors: string[] = [];
    const warnings: string[] = [];

    try {
      // Build stitched segment list
      const segments = this.buildSegmentList(request);

      // Calculate discontinuity info
      const discontinuityInfo = this.calculateDiscontinuities(segments);

      // Apply sliding window for live
      const windowedSegments = request.isLive
        ? this.applySlidingWindow(segments, discontinuityInfo)
        : segments;

      // Generate manifest
      const manifest = this.generateManifest(
        windowedSegments,
        request,
        discontinuityInfo
      );

      return {
        success: true,
        manifest,
        segments: windowedSegments,
        discontinuityCount: discontinuityInfo.count,
        mediaSequence: discontinuityInfo.mediaSequence,
        discontinuitySequence: discontinuityInfo.discontinuitySequence,
        errors,
        warnings,
      };
    } catch (error) {
      errors.push(
        `Stitching failed: ${error instanceof Error ? error.message : String(error)}`
      );
      return {
        success: false,
        discontinuityCount: 0,
        errors,
        warnings,
      };
    }
  }

  /**
   * Build the combined segment list.
   */
  private buildSegmentList(request: StitchingRequest): StitchedSegment[] {
    const segments: StitchedSegment[] = [];
    let currentTime = 0;
    let sequence = 0;
    let programDateTime = this.options.startProgramDateTime
      ? new Date(this.options.startProgramDateTime)
      : undefined;

    // Add content segments before break
    const beforeSegments = request.contentSegments.filter(
      (s) => s.position === 'before'
    );
    for (const seg of beforeSegments) {
      segments.push({
        uri: seg.uri,
        duration: seg.duration,
        mediaTime: currentTime,
        sequence: sequence++,
        isAd: false,
        discontinuity: seg.discontinuity || false,
        initSegment: seg.initSegment,
        programDateTime: programDateTime
          ? new Date(programDateTime)
          : undefined,
      });
      currentTime += seg.duration;
      if (programDateTime) {
        programDateTime = new Date(
          programDateTime.getTime() + seg.duration * 1000
        );
      }
    }

    // Add discontinuity before first ad if we have content before
    let needsDiscontinuity = beforeSegments.length > 0;

    // Add ad segments
    for (const adSeg of request.adSegments) {
      segments.push({
        uri: adSeg.uri,
        duration: adSeg.duration,
        mediaTime: currentTime,
        sequence: sequence++,
        isAd: true,
        creativeId: adSeg.creativeId,
        slotId: adSeg.slotId,
        discontinuity: needsDiscontinuity || adSeg.isFirst,
        initSegment: adSeg.initSegment,
        programDateTime: programDateTime
          ? new Date(programDateTime)
          : undefined,
      });
      currentTime += adSeg.duration;
      if (programDateTime) {
        programDateTime = new Date(
          programDateTime.getTime() + adSeg.duration * 1000
        );
      }
      // Only first segment of each creative gets discontinuity
      needsDiscontinuity = false;
    }

    // Add content segments after break
    const afterSegments = request.contentSegments.filter(
      (s) => s.position === 'after'
    );
    let firstAfter = true;
    for (const seg of afterSegments) {
      segments.push({
        uri: seg.uri,
        duration: seg.duration,
        mediaTime: currentTime,
        sequence: sequence++,
        isAd: false,
        discontinuity: firstAfter || seg.discontinuity || false,
        initSegment: seg.initSegment,
        programDateTime: programDateTime
          ? new Date(programDateTime)
          : undefined,
      });
      currentTime += seg.duration;
      if (programDateTime) {
        programDateTime = new Date(
          programDateTime.getTime() + seg.duration * 1000
        );
      }
      firstAfter = false;
    }

    return segments;
  }

  /**
   * Calculate discontinuity counts and sequences.
   */
  private calculateDiscontinuities(segments: StitchedSegment[]): {
    count: number;
    mediaSequence: number;
    discontinuitySequence: number;
    positions: number[];
  } {
    const positions: number[] = [];
    let count = 0;

    for (let i = 0; i < segments.length; i++) {
      if (segments[i].discontinuity) {
        positions.push(i);
        count++;
      }
    }

    return {
      count,
      mediaSequence: segments[0]?.sequence || 0,
      discontinuitySequence: 0, // Would be calculated from previous manifests
      positions,
    };
  }

  /**
   * Apply sliding window for live playlists.
   */
  private applySlidingWindow(
    segments: StitchedSegment[],
    _discontinuityInfo: ReturnType<typeof this.calculateDiscontinuities>
  ): StitchedSegment[] {
    if (
      this.options.slidingWindowSize <= 0 ||
      segments.length <= this.options.slidingWindowSize
    ) {
      return segments;
    }

    // Keep only the last N segments
    const start = segments.length - this.options.slidingWindowSize;
    return segments.slice(start);
  }

  /**
   * Generate the HLS manifest content.
   */
  private generateManifest(
    segments: StitchedSegment[],
    request: StitchingRequest,
    discontinuityInfo: ReturnType<typeof this.calculateDiscontinuities>
  ): string {
    const lines: string[] = [];

    // Header
    lines.push('#EXTM3U');
    lines.push('#EXT-X-VERSION:7');
    lines.push(`#EXT-X-TARGETDURATION:${this.options.targetDuration}`);
    lines.push(`#EXT-X-MEDIA-SEQUENCE:${discontinuityInfo.mediaSequence}`);

    if (discontinuityInfo.discontinuitySequence > 0) {
      lines.push(
        `#EXT-X-DISCONTINUITY-SEQUENCE:${discontinuityInfo.discontinuitySequence}`
      );
    }

    // Add DATERANGE for ad break if enabled
    if (this.options.includeDateRange && request.adSegments.length > 0) {
      const adStart = segments.find((s) => s.isAd);
      if (adStart?.programDateTime) {
        lines.push(
          `#EXT-X-DATERANGE:ID="${request.podId}",CLASS="com.apple.hls.interstitial",START-DATE="${adStart.programDateTime.toISOString()}",DURATION=${request.breakDuration}`
        );
      }
    }

    // Current init segment for CMAF
    let currentInitSegment: string | undefined;

    // Add segments
    for (const seg of segments) {
      // Add discontinuity tag
      if (seg.discontinuity) {
        lines.push('#EXT-X-DISCONTINUITY');
      }

      // Add init segment for CMAF
      if (request.container === 'cmaf' && seg.initSegment) {
        if (seg.initSegment !== currentInitSegment) {
          lines.push(`#EXT-X-MAP:URI="${seg.initSegment}"`);
          currentInitSegment = seg.initSegment;
        }
      }

      // Add program date time
      if (this.options.includeProgramDateTime && seg.programDateTime) {
        lines.push(
          `#EXT-X-PROGRAM-DATE-TIME:${seg.programDateTime.toISOString()}`
        );
      }

      // Add EXTINF and URI
      lines.push(`#EXTINF:${seg.duration.toFixed(6)},`);
      lines.push(seg.uri);
    }

    // Add endlist for VOD
    if (!request.isLive) {
      lines.push('#EXT-X-ENDLIST');
    }

    return lines.join('\n');
  }

  /**
   * Validate stitched segments for common issues.
   */
  validateStitching(segments: StitchedSegment[]): {
    valid: boolean;
    issues: string[];
  } {
    const issues: string[] = [];

    // Check for zero-duration segments
    for (let i = 0; i < segments.length; i++) {
      if (segments[i].duration <= 0) {
        issues.push(`Segment ${i} has invalid duration: ${segments[i].duration}`);
      }
    }

    // Check target duration
    const maxDuration = Math.max(...segments.map((s) => s.duration));
    if (maxDuration > this.options.targetDuration) {
      issues.push(
        `Segment duration ${maxDuration.toFixed(3)} exceeds target duration ${this.options.targetDuration}`
      );
    }

    // Check discontinuity at content/ad boundaries
    let wasAd = false;
    for (let i = 0; i < segments.length; i++) {
      const isAd = segments[i].isAd;
      if (i > 0 && isAd !== wasAd && !segments[i].discontinuity) {
        issues.push(
          `Missing discontinuity at content/ad boundary (segment ${i})`
        );
      }
      wasAd = isAd;
    }

    // Check monotonic media time
    for (let i = 1; i < segments.length; i++) {
      const expectedTime =
        segments[i - 1].mediaTime + segments[i - 1].duration;
      const tolerance = 0.1; // 100ms tolerance
      if (
        Math.abs(segments[i].mediaTime - expectedTime) > tolerance
      ) {
        issues.push(
          `Non-monotonic media time at segment ${i}: expected ${expectedTime.toFixed(3)}, got ${segments[i].mediaTime.toFixed(3)}`
        );
      }
    }

    return {
      valid: issues.length === 0,
      issues,
    };
  }

  /**
   * Update options.
   */
  updateOptions(options: Partial<HLSStitchingOptions>): void {
    this.options = { ...this.options, ...options };
  }

  /**
   * Get current options.
   */
  getOptions(): HLSStitchingOptions {
    return { ...this.options };
  }
}

export default HLSStitcher;
