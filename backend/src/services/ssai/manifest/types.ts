/**
 * SSAI Manifest Types
 *
 * Defines all types related to manifest generation for HLS and DASH.
 */

/**
 * Protocol type
 */
export type ManifestProtocol = 'hls' | 'dash';

/**
 * Container format
 */
export type ContainerFormat = 'ts' | 'cmaf' | 'fmp4';

/**
 * Stream mode
 */
export type StreamMode = 'live' | 'vod' | 'event';

/**
 * Segment information
 */
export interface SegmentInfo {
  /** Segment URI */
  uri: string;

  /** Segment duration in seconds */
  duration: number;

  /** Segment sequence number */
  sequence: number;

  /** Whether this is an ad segment */
  isAd: boolean;

  /** Associated pod ID if ad segment */
  podId?: string;

  /** Associated slot ID if ad segment */
  slotId?: string;

  /** Discontinuity before this segment */
  discontinuity?: boolean;

  /** Program date time (ISO 8601) */
  programDateTime?: string;

  /** Byte range (for fMP4) */
  byteRange?: {
    length: number;
    offset: number;
  };
}

/**
 * Rendition/Variant information
 */
export interface RenditionInfo {
  /** Rendition ID */
  id: string;

  /** Bandwidth in bits per second */
  bandwidth: number;

  /** Resolution width */
  width?: number;

  /** Resolution height */
  height?: number;

  /** Codec string */
  codecs: string;

  /** Frame rate */
  frameRate?: number;

  /** Audio group ID */
  audioGroup?: string;

  /** Subtitle group ID */
  subtitleGroup?: string;

  /** Closed captions group ID */
  closedCaptionsGroup?: string;

  /** Playlist URI */
  uri: string;
}

/**
 * Audio rendition information
 */
export interface AudioRenditionInfo {
  /** Group ID */
  groupId: string;

  /** Language */
  language: string;

  /** Name */
  name: string;

  /** Whether this is the default */
  default: boolean;

  /** Auto-select */
  autoselect: boolean;

  /** Channels (e.g., "2", "6") */
  channels?: string;

  /** URI (if not muxed) */
  uri?: string;
}

/**
 * DATERANGE cue for HLS
 */
export interface DateRangeCue {
  /** Unique ID */
  id: string;

  /** Class (e.g., "com.apple.hls.interstitial") */
  class?: string;

  /** Start date (ISO 8601) */
  startDate: string;

  /** End date (ISO 8601) */
  endDate?: string;

  /** Duration in seconds */
  duration?: number;

  /** Planned duration in seconds */
  plannedDuration?: number;

  /** SCTE-35 command in hex */
  scte35Cmd?: string;

  /** SCTE-35 out signal */
  scte35Out?: string;

  /** SCTE-35 in signal */
  scte35In?: string;

  /** Custom attributes */
  customAttributes?: Record<string, string | number>;
}

/**
 * DASH Period information
 */
export interface DASHPeriod {
  /** Period ID */
  id: string;

  /** Start time (ISO 8601 duration) */
  start: string;

  /** Duration (ISO 8601 duration) */
  duration?: string;

  /** Whether this is an ad period */
  isAd: boolean;

  /** Associated pod ID if ad period */
  podId?: string;

  /** Adaptation sets */
  adaptationSets: DASHAdaptationSet[];

  /** Event streams */
  eventStreams?: DASHEventStream[];
}

/**
 * DASH Adaptation Set
 */
export interface DASHAdaptationSet {
  /** ID */
  id: string;

  /** Content type */
  contentType: 'video' | 'audio' | 'text';

  /** MIME type */
  mimeType: string;

  /** Codecs */
  codecs?: string;

  /** Language */
  lang?: string;

  /** Segment alignment */
  segmentAlignment: boolean;

  /** Representations */
  representations: DASHRepresentation[];
}

/**
 * DASH Representation
 */
export interface DASHRepresentation {
  /** ID */
  id: string;

  /** Bandwidth */
  bandwidth: number;

  /** Width */
  width?: number;

  /** Height */
  height?: number;

  /** Codecs */
  codecs?: string;

  /** Frame rate */
  frameRate?: string;

  /** Segment template */
  segmentTemplate?: DASHSegmentTemplate;

  /** Segment list */
  segmentList?: DASHSegmentList;
}

/**
 * DASH Segment Template
 */
export interface DASHSegmentTemplate {
  /** Timescale */
  timescale: number;

  /** Presentation time offset */
  presentationTimeOffset?: number;

  /** Media template */
  media: string;

  /** Initialization template */
  initialization?: string;

  /** Start number */
  startNumber?: number;

  /** Duration */
  duration?: number;

  /** Segment timeline */
  segmentTimeline?: DASHSegmentTimeline;
}

/**
 * DASH Segment Timeline
 */
export interface DASHSegmentTimeline {
  /** S elements */
  segments: Array<{
    /** Start time (t) */
    t?: number;
    /** Duration (d) */
    d: number;
    /** Repeat count (r) */
    r?: number;
  }>;
}

/**
 * DASH Segment List
 */
export interface DASHSegmentList {
  /** Timescale */
  timescale: number;

  /** Duration */
  duration: number;

  /** Initialization URL */
  initialization?: string;

  /** Segment URLs */
  segmentUrls: Array<{
    media: string;
    mediaRange?: string;
  }>;
}

/**
 * DASH Event Stream
 */
export interface DASHEventStream {
  /** Scheme ID URI */
  schemeIdUri: string;

  /** Value */
  value?: string;

  /** Timescale */
  timescale?: number;

  /** Events */
  events: DASHEvent[];
}

/**
 * DASH Event
 */
export interface DASHEvent {
  /** Presentation time */
  presentationTime?: number;

  /** Duration */
  duration?: number;

  /** ID */
  id?: number;

  /** Message data (base64 for SCTE-35) */
  messageData?: string;
}

/**
 * Manifest generation context
 */
export interface ManifestContext {
  /** Session ID */
  sessionId: string;

  /** Manifest version */
  manifestVersion: number;

  /** Protocol */
  protocol: ManifestProtocol;

  /** Container format */
  container: ContainerFormat;

  /** Stream mode */
  mode: StreamMode;

  /** Target duration */
  targetDuration: number;

  /** Current media sequence */
  mediaSequence: number;

  /** Current discontinuity sequence */
  discontinuitySequence: number;

  /** Base URL for segments */
  segmentBaseUrl: string;

  /** Signing token */
  token: string;
}

/**
 * HLS Manifest options
 */
export interface HLSManifestOptions {
  /** Include program date time */
  includeProgramDateTime: boolean;

  /** Include DATERANGE cues */
  includeDateRange: boolean;

  /** Live sliding window size (number of segments) */
  liveWindowSize: number;

  /** Part target duration for LL-HLS */
  partTargetDuration?: number;

  /** Server control for LL-HLS */
  serverControl?: {
    canBlockReload: boolean;
    partHoldBack?: number;
    holdBack?: number;
  };
}

/**
 * DASH Manifest options
 */
export interface DASHManifestOptions {
  /** Minimum buffer time (ISO 8601 duration) */
  minBufferTime: string;

  /** Minimum update period (ISO 8601 duration) */
  minUpdatePeriod?: string;

  /** Suggested presentation delay (ISO 8601 duration) */
  suggestedPresentationDelay?: string;

  /** Time shift buffer depth (ISO 8601 duration) */
  timeShiftBufferDepth?: string;

  /** UTC timing scheme */
  utcTiming?: {
    schemeIdUri: string;
    value: string;
  };
}

/**
 * Cache policy for manifests
 */
export interface ManifestCachePolicy {
  /** Cache-Control header value */
  cacheControl: string;

  /** Vary header values */
  vary: string[];

  /** ETag */
  etag?: string;

  /** Last-Modified */
  lastModified?: string;
}

/**
 * Default cache policies
 */
export const MANIFEST_CACHE_POLICIES = {
  /** Session manifest - private, no-store */
  session: {
    cacheControl: 'private, no-store, no-cache, must-revalidate',
    vary: ['Authorization', 'Accept-Encoding'],
  } as ManifestCachePolicy,

  /** Live manifest - very short TTL */
  live: {
    cacheControl: 'private, max-age=1, stale-while-revalidate=1',
    vary: ['Authorization', 'Accept-Encoding'],
  } as ManifestCachePolicy,

  /** VOD manifest - can cache longer */
  vod: {
    cacheControl: 'private, max-age=60',
    vary: ['Authorization', 'Accept-Encoding'],
  } as ManifestCachePolicy,

  /** Segments - cacheable */
  segment: {
    cacheControl: 'public, max-age=31536000, immutable',
    vary: ['Accept-Encoding'],
  } as ManifestCachePolicy,
};

/**
 * Manifest validation result
 */
export interface ManifestValidationResult {
  /** Whether manifest is valid */
  valid: boolean;

  /** Validation errors */
  errors: ManifestValidationError[];

  /** Validation warnings */
  warnings: ManifestValidationWarning[];
}

/**
 * Manifest validation error
 */
export interface ManifestValidationError {
  /** Error code */
  code: string;

  /** Error message */
  message: string;

  /** Line number (if applicable) */
  line?: number;
}

/**
 * Manifest validation warning
 */
export interface ManifestValidationWarning {
  /** Warning code */
  code: string;

  /** Warning message */
  message: string;

  /** Line number (if applicable) */
  line?: number;
}
