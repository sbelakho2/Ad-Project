/**
 * SSAI Stitching Types
 *
 * Types and interfaces for the stitching and packaging module.
 */

export type StitchingMode = 'replacement' | 'insertion';

export interface StitchingRequest {
  /** Session ID */
  sessionId: string;

  /** Pod ID */
  podId: string;

  /** Stitching mode */
  mode: StitchingMode;

  /** Protocol */
  protocol: 'hls' | 'dash';

  /** Container */
  container: 'ts' | 'cmaf';

  /** Content segments to stitch around */
  contentSegments: ContentSegment[];

  /** Ad segments to stitch in */
  adSegments: AdSegment[];

  /** Break position (media time in seconds) */
  breakPosition: number;

  /** Break duration (seconds) */
  breakDuration: number;

  /** Whether this is live or VOD */
  isLive: boolean;
}

export interface ContentSegment {
  /** Segment URI */
  uri: string;

  /** Segment duration */
  duration: number;

  /** Start time (media time) */
  startTime: number;

  /** Sequence number */
  sequence: number;

  /** Whether this is before or after the break */
  position: 'before' | 'after';

  /** Discontinuity before this segment */
  discontinuity?: boolean;

  /** Init segment URI (for CMAF) */
  initSegment?: string;

  /** Program date time */
  programDateTime?: Date;
}

export interface AdSegment {
  /** Segment URI */
  uri: string;

  /** Segment duration */
  duration: number;

  /** Creative ID this segment belongs to */
  creativeId: string;

  /** Slot ID */
  slotId: string;

  /** Sequence within the creative */
  creativeSequence: number;

  /** Total segments in creative */
  creativeTotalSegments: number;

  /** Init segment URI (for CMAF) */
  initSegment?: string;

  /** Whether this is first segment (needs discontinuity) */
  isFirst: boolean;

  /** Whether this is last segment of this creative */
  isLast: boolean;
}

export interface StitchingResult {
  /** Success status */
  success: boolean;

  /** Stitched manifest content */
  manifest?: string;

  /** Updated segment list */
  segments?: StitchedSegment[];

  /** Discontinuity count */
  discontinuityCount: number;

  /** Media sequence number for live */
  mediaSequence?: number;

  /** Discontinuity sequence number */
  discontinuitySequence?: number;

  /** Errors */
  errors: string[];

  /** Warnings */
  warnings: string[];
}

export interface StitchedSegment {
  /** Segment URI */
  uri: string;

  /** Duration */
  duration: number;

  /** Media time */
  mediaTime: number;

  /** Sequence number */
  sequence: number;

  /** Is ad segment */
  isAd: boolean;

  /** Creative ID (if ad) */
  creativeId?: string;

  /** Slot ID (if ad) */
  slotId?: string;

  /** Discontinuity before */
  discontinuity: boolean;

  /** Init segment (for CMAF) */
  initSegment?: string;

  /** Program date time */
  programDateTime?: Date;
}

export interface DRMPolicy {
  /** Whether content is encrypted */
  contentEncrypted: boolean;

  /** Whether ads are encrypted */
  adsEncrypted: boolean;

  /** DRM scheme (widevine, fairplay, playready) */
  scheme?: string;

  /** Key continuity at boundaries */
  keyContinuity: boolean;

  /** Key rotation policy */
  keyRotation?: 'per-segment' | 'per-period' | 'none';
}

export interface AudioConfig {
  /** Audio layout (stereo, 5.1, etc.) */
  layout: 'stereo' | '5.1' | '7.1' | 'mono';

  /** Sample rate */
  sampleRate: number;

  /** Bit depth */
  bitDepth: number;

  /** Loudness normalization target (LUFS) */
  loudnessTarget: number;
}

// CaptionConfig is exported from CaptionHandler.ts to avoid duplicate

export interface SegmentAlignment {
  /** Target segment duration */
  targetDuration: number;

  /** Tolerance for segment duration variance */
  durationTolerance: number;

  /** Keyframe alignment required */
  keyframeAlignment: boolean;

  /** GOP duration (for alignment) */
  gopDuration?: number;
}
