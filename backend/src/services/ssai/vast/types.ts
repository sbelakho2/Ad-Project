/**
 * VAST/VMAP Type Definitions
 *
 * Comprehensive types for VAST 2.0/3.0/4.0/4.1/4.2 and VMAP 1.0
 * Based on IAB Tech Lab specifications:
 * - VAST: https://iabtechlab.com/standards/vast/
 * - VMAP: https://iabtechlab.com/standards/vmap/
 */

// =============================================================================
// VAST Core Types (4.2 with backward compatibility)
// =============================================================================

/**
 * Root VAST document
 */
export interface VASTDocument {
  /** VAST version (2.0, 3.0, 4.0, 4.1, 4.2) */
  version: VASTVersion;

  /** List of ads in the document */
  ads: VASTAd[];

  /** Error URLs to call if no ads are returned */
  errors: string[];

  /** Raw XML for debugging */
  rawXml?: string;

  /** Parse timestamp */
  parsedAt: Date;
}

export type VASTVersion = '2.0' | '3.0' | '4.0' | '4.1' | '4.2';

/**
 * VAST Ad - can be InLine or Wrapper
 */
export interface VASTAd {
  /** Ad ID (required) */
  id: string;

  /** Ad sequence number for ad pods */
  sequence?: number;

  /** Whether ad is conditional */
  conditionalAd?: boolean;

  /** Ad type: inline contains creative, wrapper redirects */
  type: 'inline' | 'wrapper';

  /** InLine ad data (if type === 'inline') */
  inLine?: VASTInLine;

  /** Wrapper data (if type === 'wrapper') */
  wrapper?: VASTWrapper;
}

/**
 * InLine ad - contains the actual creative
 */
export interface VASTInLine {
  /** Ad system info */
  adSystem: VASTAdSystem;

  /** Ad title */
  adTitle: string;

  /** Ad description */
  description?: string;

  /** Advertiser name */
  advertiser?: string;

  /** Ad serving ID for tracking */
  adServingId?: string;

  /** Category information */
  categories: VASTCategory[];

  /** Pricing information */
  pricing?: VASTPricing;

  /** Survey URL */
  survey?: string;

  /** Impression tracking URLs */
  impressions: VASTImpression[];

  /** Error tracking URLs */
  errors: string[];

  /** Viewable impression tracking (VAST 4.x) */
  viewableImpression?: VASTViewableImpression;

  /** Creatives in this ad */
  creatives: VASTCreative[];

  /** Extensions */
  extensions: VASTExtension[];

  /** Ad verification scripts (VAST 4.x) */
  adVerifications: VASTVerification[];
}

/**
 * Wrapper ad - redirects to another VAST
 */
export interface VASTWrapper {
  /** Ad system info */
  adSystem: VASTAdSystem;

  /** URL to follow for the wrapped VAST */
  vastAdTagUri: string;

  /** Whether to follow additional wrappers */
  followAdditionalWrappers?: boolean;

  /** Whether creative can be reused */
  allowMultipleAds?: boolean;

  /** Fallback if wrapper fails */
  fallbackOnNoAd?: boolean;

  /** Impression tracking URLs (accumulated through chain) */
  impressions: VASTImpression[];

  /** Error tracking URLs */
  errors: string[];

  /** Viewable impression tracking */
  viewableImpression?: VASTViewableImpression;

  /** Creative elements for tracking accumulation */
  creatives: VASTCreative[];

  /** Extensions */
  extensions: VASTExtension[];

  /** Ad verification scripts */
  adVerifications: VASTVerification[];
}

// =============================================================================
// Ad System and Metadata
// =============================================================================

export interface VASTAdSystem {
  /** Ad system name */
  name: string;

  /** Ad system version */
  version?: string;
}

export interface VASTCategory {
  /** Category authority (e.g., 'IAB') */
  authority?: string;

  /** Category value */
  value: string;
}

export interface VASTPricing {
  /** Price model: CPM, CPC, CPE, CPV */
  model: 'cpm' | 'cpc' | 'cpe' | 'cpv';

  /** Currency code (ISO 4217) */
  currency: string;

  /** Price value */
  value: number;
}

export interface VASTImpression {
  /** Impression ID */
  id?: string;

  /** Tracking URL */
  url: string;
}

// =============================================================================
// Viewable Impression (VAST 4.x)
// =============================================================================

export interface VASTViewableImpression {
  /** Viewable tracking URL */
  viewable?: string[];

  /** Not viewable tracking URL */
  notViewable?: string[];

  /** Undetermined viewability tracking URL */
  viewUndetermined?: string[];
}

// =============================================================================
// Creatives
// =============================================================================

export interface VASTCreative {
  /** Creative ID */
  id?: string;

  /** Ad ID reference */
  adId?: string;

  /** Creative sequence */
  sequence?: number;

  /** API framework requirements */
  apiFramework?: string;

  /** Universal Ad ID (VAST 4.x) */
  universalAdIds: VASTUniversalAdId[];

  /** Creative type */
  type: 'linear' | 'nonlinear' | 'companion';

  /** Linear creative data */
  linear?: VASTLinear;

  /** NonLinear creative data */
  nonLinearAds?: VASTNonLinear[];

  /** Companion ads */
  companionAds?: VASTCompanionAds;
}

export interface VASTUniversalAdId {
  /** ID registry */
  idRegistry: string;

  /** ID value */
  idValue: string;
}

// =============================================================================
// Linear Creatives (Video/Audio)
// =============================================================================

export interface VASTLinear {
  /** Skip offset (e.g., '00:00:05', '10%') */
  skipOffset?: string;

  /** Duration in seconds */
  duration: number;

  /** Ad parameters for custom player */
  adParameters?: VASTAdParameters;

  /** Media files */
  mediaFiles: VASTMediaFile[];

  /** Mezzanine file (VAST 4.x) */
  mezzanine?: VASTMezzanine;

  /** Interactive creative files (VAST 4.1+) */
  interactiveCreativeFiles?: VASTInteractiveCreativeFile[];

  /** Closed caption files (VAST 4.1+) */
  closedCaptionFiles?: VASTClosedCaptionFile[];

  /** Tracking events */
  trackingEvents: VASTTrackingEvent[];

  /** Video clicks */
  videoClicks?: VASTVideoClicks;

  /** Icons */
  icons: VASTIcon[];
}

export interface VASTAdParameters {
  /** XML encoded flag */
  xmlEncoded?: boolean;

  /** Parameters data */
  data: string;
}

export interface VASTMediaFile {
  /** Media file ID */
  id?: string;

  /** Delivery type */
  delivery: 'streaming' | 'progressive';

  /** MIME type */
  type: string;

  /** Codec */
  codec?: string;

  /** Bitrate in kbps */
  bitrate?: number;

  /** Minimum bitrate */
  minBitrate?: number;

  /** Maximum bitrate */
  maxBitrate?: number;

  /** Width in pixels */
  width: number;

  /** Height in pixels */
  height: number;

  /** Scalable flag */
  scalable?: boolean;

  /** Maintain aspect ratio */
  maintainAspectRatio?: boolean;

  /** API framework */
  apiFramework?: string;

  /** File size in bytes */
  fileSize?: number;

  /** Media type: 2D or 3D */
  mediaType?: '2D' | '3D';

  /** Media file URL */
  url: string;
}

export interface VASTMezzanine {
  /** Delivery type */
  delivery: 'streaming' | 'progressive';

  /** MIME type */
  type: string;

  /** Width */
  width: number;

  /** Height */
  height: number;

  /** File size */
  fileSize?: number;

  /** Media type */
  mediaType?: '2D' | '3D';

  /** URL */
  url: string;
}

export interface VASTInteractiveCreativeFile {
  /** MIME type */
  type: string;

  /** API framework (e.g., 'SIMID') */
  apiFramework: string;

  /** Variable duration flag */
  variableDuration?: boolean;

  /** URL */
  url: string;
}

export interface VASTClosedCaptionFile {
  /** MIME type */
  type: string;

  /** Language (ISO 639-1) */
  language: string;

  /** URL */
  url: string;
}

// =============================================================================
// Tracking Events
// =============================================================================

export type VASTTrackingEventType =
  | 'creativeView'
  | 'start'
  | 'firstQuartile'
  | 'midpoint'
  | 'thirdQuartile'
  | 'complete'
  | 'mute'
  | 'unmute'
  | 'pause'
  | 'rewind'
  | 'resume'
  | 'fullscreen'
  | 'exitFullscreen'
  | 'expand'
  | 'collapse'
  | 'loaded'
  | 'playerExpand'
  | 'playerCollapse'
  | 'skip'
  | 'progress'
  | 'closeLinear'
  | 'close'
  | 'acceptInvitationLinear'
  | 'acceptInvitation'
  | 'adExpand'
  | 'adCollapse'
  | 'minimize'
  | 'overlayViewDuration'
  | 'otherAdInteraction';

export interface VASTTrackingEvent {
  /** Event type */
  event: VASTTrackingEventType;

  /** Offset for progress events (time or percentage) */
  offset?: string;

  /** Tracking URL */
  url: string;
}

// =============================================================================
// Video Clicks
// =============================================================================

export interface VASTVideoClicks {
  /** Click through URL */
  clickThrough?: VASTClickThrough;

  /** Click tracking URLs */
  clickTrackings: VASTClickTracking[];

  /** Custom click URLs */
  customClicks: VASTCustomClick[];
}

export interface VASTClickThrough {
  /** ID */
  id?: string;

  /** URL */
  url: string;
}

export interface VASTClickTracking {
  /** ID */
  id?: string;

  /** URL */
  url: string;
}

export interface VASTCustomClick {
  /** ID */
  id?: string;

  /** URL */
  url: string;
}

// =============================================================================
// Icons
// =============================================================================

export interface VASTIcon {
  /** Icon program (e.g., 'AdChoices') */
  program: string;

  /** Width */
  width: number;

  /** Height */
  height: number;

  /** X position */
  xPosition: string | number;

  /** Y position */
  yPosition: string | number;

  /** Duration */
  duration?: number;

  /** Offset */
  offset?: string;

  /** API framework */
  apiFramework?: string;

  /** Pixels per inch */
  pxratio?: number;

  /** Static resource */
  staticResource?: VASTStaticResource;

  /** IFrame resource */
  iFrameResource?: string;

  /** HTML resource */
  htmlResource?: string;

  /** Icon clicks */
  iconClicks?: VASTIconClicks;

  /** Icon view tracking */
  iconViewTrackings: string[];
}

export interface VASTStaticResource {
  /** MIME type */
  creativeType: string;

  /** URL */
  url: string;
}

export interface VASTIconClicks {
  /** Click through */
  iconClickThrough?: string;

  /** Click tracking */
  iconClickTrackings: string[];

  /** Fallback images */
  iconClickFallbackImages?: VASTIconClickFallbackImage[];
}

export interface VASTIconClickFallbackImage {
  /** Width */
  width: number;

  /** Height */
  height: number;

  /** Alt text */
  altText?: string;

  /** URL */
  url: string;
}

// =============================================================================
// NonLinear Creatives
// =============================================================================

export interface VASTNonLinear {
  /** ID */
  id?: string;

  /** Width */
  width: number;

  /** Height */
  height: number;

  /** Expanded width */
  expandedWidth?: number;

  /** Expanded height */
  expandedHeight?: number;

  /** Scalable */
  scalable?: boolean;

  /** Maintain aspect ratio */
  maintainAspectRatio?: boolean;

  /** Minimum suggested duration */
  minSuggestedDuration?: string;

  /** API framework */
  apiFramework?: string;

  /** Static resource */
  staticResource?: VASTStaticResource;

  /** IFrame resource */
  iFrameResource?: string;

  /** HTML resource */
  htmlResource?: string;

  /** Ad parameters */
  adParameters?: VASTAdParameters;

  /** Click through */
  nonLinearClickThrough?: string;

  /** Click tracking */
  nonLinearClickTrackings: string[];
}

// =============================================================================
// Companion Ads
// =============================================================================

export interface VASTCompanionAds {
  /** Required attribute */
  required?: 'all' | 'any' | 'none';

  /** Companion creatives */
  companions: VASTCompanion[];
}

export interface VASTCompanion {
  /** ID */
  id?: string;

  /** Width */
  width: number;

  /** Height */
  height: number;

  /** Asset width */
  assetWidth?: number;

  /** Asset height */
  assetHeight?: number;

  /** Expanded width */
  expandedWidth?: number;

  /** Expanded height */
  expandedHeight?: number;

  /** API framework */
  apiFramework?: string;

  /** Ad slot ID */
  adSlotId?: string;

  /** Pixels per inch */
  pxratio?: number;

  /** Rendering mode */
  renderingMode?: 'default' | 'end-card' | 'concurrent';

  /** Static resource */
  staticResource?: VASTStaticResource;

  /** IFrame resource */
  iFrameResource?: string;

  /** HTML resource */
  htmlResource?: string;

  /** Ad parameters */
  adParameters?: VASTAdParameters;

  /** Alt text */
  altText?: string;

  /** Click through */
  companionClickThrough?: string;

  /** Click tracking */
  companionClickTrackings: string[];

  /** Tracking events */
  trackingEvents: VASTTrackingEvent[];
}

// =============================================================================
// Verification (VAST 4.x)
// =============================================================================

export interface VASTVerification {
  /** Vendor name */
  vendor?: string;

  /** JavaScript resource */
  javaScriptResource?: VASTJavaScriptResource;

  /** Executable resource */
  executableResource?: VASTExecutableResource;

  /** Tracking events for verification */
  trackingEvents: VASTVerificationTrackingEvent[];

  /** Verification parameters */
  verificationParameters?: string;
}

export interface VASTJavaScriptResource {
  /** API framework */
  apiFramework?: string;

  /** Browser optional */
  browserOptional?: boolean;

  /** URL */
  url: string;
}

export interface VASTExecutableResource {
  /** API framework */
  apiFramework?: string;

  /** MIME type */
  type?: string;

  /** URL */
  url: string;
}

export interface VASTVerificationTrackingEvent {
  /** Event type */
  event: 'verificationNotExecuted';

  /** Reason */
  reason?: string;

  /** URL */
  url: string;
}

// =============================================================================
// Extensions
// =============================================================================

export interface VASTExtension {
  /** Extension type */
  type?: string;

  /** Raw XML content */
  content: string;

  /** Parsed data (if known extension type) */
  data?: Record<string, unknown>;
}

// =============================================================================
// VMAP Types (Video Multiple Ad Playlist)
// =============================================================================

/**
 * Root VMAP document
 */
export interface VMAPDocument {
  /** VMAP version */
  version: '1.0';

  /** Ad breaks in the playlist */
  adBreaks: VMAPAdBreak[];

  /** Extensions */
  extensions: VMAPExtension[];

  /** Raw XML for debugging */
  rawXml?: string;

  /** Parse timestamp */
  parsedAt: Date;
}

/**
 * VMAP Ad Break
 */
export interface VMAPAdBreak {
  /** Time offset (parsed) */
  timeOffset: VMAPTimeOffset;

  /** Break type */
  breakType: 'linear' | 'nonlinear' | 'display';

  /** Break ID */
  breakId?: string;

  /** Repeat after (seconds) */
  repeatAfter?: number;

  /** Ad source */
  adSource?: VMAPAdSource;

  /** Tracking events */
  trackingEvents: VMAPTrackingEvent[];

  /** Extensions */
  extensions: VMAPExtension[];
}

/**
 * VMAP Time Offset (parsed)
 */
export type VMAPTimeOffset =
  | { type: 'start' }
  | { type: 'end' }
  | { type: 'time'; seconds: number }
  | { type: 'percentage'; percent: number }
  | { type: 'position'; position: number };

export type VMAPAdSource =
  | VMAPInlineAdSource
  | VMAPRedirectAdSource
  | VMAPCustomAdSource;

export interface VMAPAdSourceBase {
  /** Source ID */
  id?: string;

  /** Allow multiple ads */
  allowMultipleAds?: boolean;

  /** Follow redirects */
  followRedirects?: boolean;
}

export interface VMAPInlineAdSource extends VMAPAdSourceBase {
  type: 'inline';

  /** Inline VAST data */
  vastData?: VASTDocument;
}

export interface VMAPRedirectAdSource extends VMAPAdSourceBase {
  type: 'redirect';

  /** Ad tag URI */
  adTagUri: string;

  /** Template type */
  templateType?: string;

  /** Resolved VAST (if fetched) */
  resolvedVast?: VASTDocument;
}

export interface VMAPCustomAdSource extends VMAPAdSourceBase {
  type: 'custom';

  /** Template type */
  templateType?: string;

  /** Custom data content */
  customData: string;
}

export type VMAPTrackingEventType =
  | 'breakStart'
  | 'breakEnd'
  | 'error';

export interface VMAPTrackingEvent {
  /** Event type */
  event: VMAPTrackingEventType;

  /** Tracking URL */
  url: string;
}

export interface VMAPExtension {
  /** Extension type */
  type?: string;

  /** Raw content */
  content: string;
}

// =============================================================================
// Parser Configuration
// =============================================================================

export interface VASTParserConfig {
  /** Maximum wrapper depth to follow */
  maxWrapperDepth: number;

  /** Timeout for fetching individual wrappers (ms) */
  wrapperTimeoutMs: number;

  /** Maximum total time for all wrapper resolution (ms) - DoS protection */
  maxTotalWrapperTimeMs: number;

  /** Whether to resolve wrappers automatically */
  resolveWrappers: boolean;

  /** Whether to accumulate tracking URLs through wrapper chain */
  accumulateTracking: boolean;

  /** 
   * Whether to treat wrapper fetch failures as warnings instead of errors.
   * When true, failed wrappers are skipped but parsing continues.
   * When false, any wrapper failure (without fallbackOnNoAd) fails the parse.
   * Default: false (strict per IAB spec)
   */
  gracefulWrapperFailure: boolean;

  /** Allowed VAST versions */
  allowedVersions: VASTVersion[];

  /** Whether to validate against schema */
  validateSchema: boolean;

  /** Whether to parse extensions */
  parseExtensions: boolean;

  /** Maximum media files per ad (DoS protection) - excess files are ignored */
  maxMediaFilesPerAd: number;

  /** Maximum companion ads per creative (DoS protection) - excess companions are ignored */
  maxCompanionsPerCreative: number;

  /** Custom extension parsers */
  extensionParsers?: Record<string, (content: string) => unknown>;

  /** HTTP client options */
  httpOptions?: {
    timeout: number;
    headers?: Record<string, string>;
    followRedirects?: boolean;
  };
}

export const DEFAULT_VAST_PARSER_CONFIG: VASTParserConfig = {
  maxWrapperDepth: 5,
  wrapperTimeoutMs: 5000,
  maxTotalWrapperTimeMs: 30000, // 30 seconds total for all wrappers
  maxMediaFilesPerAd: 50, // Reasonable limit per ad (DoS protection)
  maxCompanionsPerCreative: 20, // Reasonable limit per creative (DoS protection)
  resolveWrappers: true,
  accumulateTracking: true,
  gracefulWrapperFailure: false, // Strict mode by default (per IAB spec)
  allowedVersions: ['2.0', '3.0', '4.0', '4.1', '4.2'],
  validateSchema: false,
  parseExtensions: true,
  httpOptions: {
    timeout: 10000,
    followRedirects: true,
  },
};

export interface VMAPParserConfig {
  /** Maximum wrapper depth to follow */
  maxWrapperDepth: number;

  /** Timeout for fetching wrappers/VAST (ms) */
  wrapperTimeoutMs: number;

  /** Whether to resolve VAST ad tags */
  resolveVAST: boolean;

  /** Whether to parse extensions */
  parseExtensions: boolean;
}

export const DEFAULT_VMAP_PARSER_CONFIG: VMAPParserConfig = {
  maxWrapperDepth: 5,
  wrapperTimeoutMs: 5000,
  resolveVAST: false,
  parseExtensions: true,
};

// =============================================================================
// Parser Results
// =============================================================================

export interface VASTParseResult {
  /** Parsed VAST document */
  document: VASTDocument | null;

  /** Whether parsing was successful */
  success: boolean;

  /** Errors encountered */
  errors: VASTError[];

  /** Warnings */
  warnings: VASTWarning[];

  /** Wrapper chain (if resolved) */
  wrapperChain?: WrapperChainEntry[];

  /** Parse duration in ms */
  parseTimeMs: number;
}

export interface VMAPParseResult {
  /** Parsed VMAP document */
  document: VMAPDocument | null;

  /** Whether parsing was successful */
  success: boolean;

  /** Errors encountered */
  errors: VMAPError[];

  /** Warnings */
  warnings: VMAPWarning[];

  /** Parse duration in ms */
  parseTimeMs: number;
}

export interface WrapperChainEntry {
  /** Depth in chain (0 = outermost) */
  depth: number;

  /** Ad system name */
  adSystem: string;

  /** VAST tag URI */
  vastTagUri: string;

  /** Response time in ms */
  responseTimeMs: number;
}

export interface VASTError {
  /** Error code */
  code: VASTErrorCode;

  /** Error message */
  message: string;

  /** Additional context */
  context?: Record<string, unknown>;
}

export interface VASTWarning {
  /** Warning code */
  code: string;

  /** Warning message */
  message: string;

  /** Severity */
  severity: 'low' | 'medium' | 'high';

  /** Optional context data */
  context?: Record<string, unknown>;
}

export type VMAPErrorCode =
  | 'XML_PARSE_ERROR'
  | 'INVALID_VMAP'
  | 'INVALID_AD_BREAK'
  | 'INVALID_TIME_OFFSET'
  | 'VAST_FETCH_FAILED'
  | 'VAST_PARSE_FAILED'
  | 'INTERNAL_ERROR';

export interface VMAPError {
  /** Error code */
  code: VMAPErrorCode;

  /** Error message */
  message: string;

  /** Additional context */
  context?: Record<string, unknown>;
}

export interface VMAPWarning {
  /** Warning code */
  code: string;

  /** Warning message */
  message: string;

  /** Severity */
  severity: 'low' | 'medium' | 'high';
}

/**
 * VAST Error Codes (per IAB spec)
 */
export enum VASTErrorCode {
  // XML Parsing Errors
  XML_PARSE_ERROR = 100,
  VAST_SCHEMA_VALIDATION_ERROR = 101,

  // Version Errors
  UNSUPPORTED_VAST_VERSION = 102,

  // Wrapper Errors
  WRAPPER_LIMIT_REACHED = 300,
  WRAPPER_TIMEOUT = 301,
  WRAPPER_NO_ADS = 302,
  WRAPPER_FETCH_ERROR = 303,

  // Linear Errors
  NO_MEDIA_FILE = 400,
  NO_SUPPORTED_MEDIA_FILE = 401,
  MEDIA_TIMEOUT = 402,
  MEDIA_LOAD_ERROR = 403,
  MEDIA_DISPLAY_ERROR = 405,

  // Companion Errors
  COMPANION_REQUIRED_BUT_UNAVAILABLE = 600,

  // NonLinear Errors
  NONLINEAR_RESOURCE_ERROR = 700,

  // General Errors
  UNDEFINED_ERROR = 900,
  GENERAL_WRAPPER_ERROR = 901,
  GENERAL_LINEAR_ERROR = 902,
  GENERAL_NONLINEAR_ERROR = 903,
  GENERAL_COMPANION_ERROR = 904,
}

// =============================================================================
// Utility Types
// =============================================================================

/**
 * Normalized media file with selection score
 */
export interface NormalizedMediaFile extends VASTMediaFile {
  /** Selection score based on quality/compatibility */
  score: number;

  /** Derived container type */
  container: 'mp4' | 'webm' | 'hls' | 'dash' | 'unknown';

  /** Derived video codec */
  videoCodec: string | null;

  /** Derived audio codec */
  audioCodec: string | null;

  /** Aspect ratio */
  aspectRatio: number;
}

/**
 * Flattened tracking URLs from a resolved VAST
 */
export interface FlattenedTracking {
  /** Impression URLs */
  impressions: string[];

  /** Error URLs */
  errors: string[];

  /** Tracking by event type */
  events: Record<VASTTrackingEventType, string[]>;

  /** Click through URL */
  clickThrough: string | null;

  /** Click tracking URLs */
  clickTrackings: string[];

  /** Verification scripts */
  verifications: VASTVerification[];

  /** Viewable impression */
  viewableImpression: VASTViewableImpression | null;
}

/**
 * Creative ready for SSAI stitching
 */
export interface SSAIReadyCreative {
  /** Creative ID */
  id: string;

  /** Duration in seconds */
  duration: number;

  /** Selected media file */
  mediaFile: NormalizedMediaFile;

  /** Alternative media files (sorted by preference) */
  alternativeMediaFiles: NormalizedMediaFile[];

  /** Flattened tracking */
  tracking: FlattenedTracking;

  /** Ad system */
  adSystem: VASTAdSystem;

  /** Universal ad IDs */
  universalAdIds: VASTUniversalAdId[];

  /** Skip offset in seconds (null = not skippable) */
  skipOffsetSeconds: number | null;

  /** Companion ads */
  companions: VASTCompanion[];

  /** Icons */
  icons: VASTIcon[];

  /** Extensions */
  extensions: VASTExtension[];

  /** Original ad ID */
  adId: string;

  /** Sequence number */
  sequence?: number;
}

export default {
  VASTErrorCode,
  DEFAULT_VAST_PARSER_CONFIG,
};
