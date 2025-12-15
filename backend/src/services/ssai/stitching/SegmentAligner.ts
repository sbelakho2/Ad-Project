/**
 * Segment Aligner
 *
 * Ensures consistent fragment/segment durations and proper
 * audio/video alignment at splice boundaries.
 */

export interface SegmentInfo {
  uri: string;
  duration: number;
  startTime: number;
  endTime: number;
  isKeyframe: boolean;
  hasAudio: boolean;
  hasVideo: boolean;
  audioDuration?: number;
  videoDuration?: number;
}

export interface AlignmentConfig {
  /** Target segment duration (seconds) */
  targetDuration: number;

  /** Maximum allowed duration variance (seconds) */
  maxVariance: number;

  /** Maximum allowed audio/video drift (seconds) */
  maxAvDrift: number;

  /** Require keyframe at segment start */
  requireKeyframe: boolean;

  /** GOP size in frames (for optimization) */
  gopSize: number;

  /** Frame rate */
  frameRate: number;
}

export interface AlignmentResult {
  aligned: boolean;
  segments: SegmentInfo[];
  adjustments: AlignmentAdjustment[];
  warnings: string[];
  errors: string[];
}

export interface AlignmentAdjustment {
  segmentIndex: number;
  originalDuration: number;
  adjustedDuration: number;
  reason: string;
}

export interface SplicePoint {
  position: number;
  isKeyframe: boolean;
  audioSyncOffset: number;
  videoSyncOffset: number;
}

/**
 * Segment Aligner for ensuring proper segment boundaries.
 */
export class SegmentAligner {
  private config: AlignmentConfig;

  constructor(config?: Partial<AlignmentConfig>) {
    this.config = {
      targetDuration: 6,
      maxVariance: 0.5,
      maxAvDrift: 0.04, // 40ms
      requireKeyframe: true,
      gopSize: 60, // 2 seconds at 30fps
      frameRate: 30,
      ...config,
    };
  }

  /**
   * Align segments for consistent duration.
   */
  alignSegments(segments: SegmentInfo[]): AlignmentResult {
    const result: AlignmentResult = {
      aligned: true,
      segments: [],
      adjustments: [],
      warnings: [],
      errors: [],
    };

    for (let i = 0; i < segments.length; i++) {
      const segment = { ...segments[i] };
      const adjustment = this.checkAndAdjustSegment(segment, i);

      if (adjustment) {
        result.adjustments.push(adjustment);
        segment.duration = adjustment.adjustedDuration;
        segment.endTime = segment.startTime + segment.duration;
      }

      // Check keyframe requirement
      if (this.config.requireKeyframe && !segment.isKeyframe && i > 0) {
        result.warnings.push(
          `Segment ${i} does not start with keyframe`
        );
      }

      // Check A/V alignment
      const avDrift = this.checkAvAlignment(segment);
      if (Math.abs(avDrift) > this.config.maxAvDrift) {
        result.warnings.push(
          `Segment ${i} has A/V drift of ${(avDrift * 1000).toFixed(1)}ms`
        );
      }

      result.segments.push(segment);
    }

    // Validate overall alignment
    if (result.errors.length > 0) {
      result.aligned = false;
    }

    return result;
  }

  /**
   * Check and potentially adjust segment duration.
   */
  private checkAndAdjustSegment(
    segment: SegmentInfo,
    index: number
  ): AlignmentAdjustment | null {
    const variance = Math.abs(segment.duration - this.config.targetDuration);

    if (variance <= this.config.maxVariance) {
      return null;
    }

    // Calculate adjusted duration (snap to nearest GOP boundary)
    const gopDuration = this.config.gopSize / this.config.frameRate;
    const gopCount = Math.round(segment.duration / gopDuration);
    const adjustedDuration = gopCount * gopDuration;

    return {
      segmentIndex: index,
      originalDuration: segment.duration,
      adjustedDuration,
      reason: `Duration variance ${variance.toFixed(3)}s exceeds max ${this.config.maxVariance}s`,
    };
  }

  /**
   * Check audio/video alignment within segment.
   */
  private checkAvAlignment(segment: SegmentInfo): number {
    if (!segment.audioDuration || !segment.videoDuration) {
      return 0;
    }

    return segment.audioDuration - segment.videoDuration;
  }

  /**
   * Find optimal splice point near target position.
   */
  findSplicePoint(
    segments: SegmentInfo[],
    targetPosition: number
  ): SplicePoint | null {
    // Find segment containing target position
    let segmentIndex = -1;
    let positionInSegment = 0;

    let currentTime = 0;
    for (let i = 0; i < segments.length; i++) {
      if (currentTime + segments[i].duration > targetPosition) {
        segmentIndex = i;
        positionInSegment = targetPosition - currentTime;
        break;
      }
      currentTime += segments[i].duration;
    }

    if (segmentIndex === -1) {
      return null;
    }

    // Find nearest keyframe boundary
    const gopDuration = this.config.gopSize / this.config.frameRate;
    const nearestGopBoundary = Math.round(positionInSegment / gopDuration) * gopDuration;
    const actualPosition = currentTime + nearestGopBoundary;

    return {
      position: actualPosition,
      isKeyframe: true, // GOP boundary should have keyframe
      audioSyncOffset: 0,
      videoSyncOffset: 0,
    };
  }

  /**
   * Validate splice boundary alignment.
   */
  validateSpliceBoundary(
    contentSegments: SegmentInfo[],
    adSegments: SegmentInfo[],
    splicePosition: number
  ): { valid: boolean; issues: string[] } {
    const issues: string[] = [];

    // Check content ends at splice point
    const lastContentSegment = contentSegments[contentSegments.length - 1];
    if (lastContentSegment) {
      const contentEnd = lastContentSegment.endTime;
      const gap = Math.abs(contentEnd - splicePosition);

      if (gap > this.config.maxVariance) {
        issues.push(
          `Content ends ${gap.toFixed(3)}s from splice point`
        );
      }
    }

    // Check first ad segment
    if (adSegments.length > 0) {
      const firstAdSegment = adSegments[0];

      if (this.config.requireKeyframe && !firstAdSegment.isKeyframe) {
        issues.push('First ad segment does not start with keyframe');
      }
    }

    return {
      valid: issues.length === 0,
      issues,
    };
  }

  /**
   * Optimize segments for one GOP per fragment.
   */
  optimizeForGop(segments: SegmentInfo[]): SegmentInfo[] {
    const gopDuration = this.config.gopSize / this.config.frameRate;
    const optimized: SegmentInfo[] = [];

    for (const segment of segments) {
      // Split segment into GOP-aligned fragments
      const gopCount = Math.round(segment.duration / gopDuration);

      if (gopCount <= 1) {
        optimized.push(segment);
        continue;
      }

      // Create GOP-aligned fragments
      let currentStart = segment.startTime;
      for (let i = 0; i < gopCount; i++) {
        optimized.push({
          uri: `${segment.uri}#gop=${i}`,
          duration: gopDuration,
          startTime: currentStart,
          endTime: currentStart + gopDuration,
          isKeyframe: true,
          hasAudio: segment.hasAudio,
          hasVideo: segment.hasVideo,
        });
        currentStart += gopDuration;
      }
    }

    return optimized;
  }

  /**
   * Get configuration.
   */
  getConfig(): AlignmentConfig {
    return { ...this.config };
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<AlignmentConfig>): void {
    this.config = { ...this.config, ...config };
  }
}

export default SegmentAligner;
