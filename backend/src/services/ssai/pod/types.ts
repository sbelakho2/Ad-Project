/**
 * SSAI Pod Decisioning Types
 *
 * Types and interfaces for the pod decisioning module.
 */

export interface PodDecisionRequest {
  /** Pod ID from pod planner */
  podId: string;

  /** Break ID */
  breakId: string;

  /** Session ID */
  sessionId: string;

  /** Tenant ID */
  tenantId: string;

  /** Slots to fill */
  slots: SlotRequest[];

  /** Session context */
  context: DecisionContext;

  /** Format requirements */
  formatRequirements: FormatRequirements;

  /** Deadline for decision (ISO timestamp) */
  deadline: string;
}

export interface SlotRequest {
  /** Slot ID */
  slotId: string;

  /** Slot duration in seconds */
  duration: number;

  /** Position in pod (0-indexed) */
  position: number;

  /** Minimum duration (if flexible) */
  minDuration?: number;

  /** Maximum duration (if flexible) */
  maxDuration?: number;

  /** Whether this slot allows fallback to slate */
  allowSlate: boolean;
}

export interface DecisionContext {
  /** Device type */
  deviceType: 'ctv' | 'mobile' | 'desktop' | 'tablet';

  /** Player family */
  playerFamily?: string;

  /** Geography */
  geography?: {
    country?: string;
    region?: string;
    dma?: string;
  };

  /** Content context */
  content?: {
    genre?: string[];
    rating?: string;
    channelId?: string;
    programId?: string;
  };

  /** User segments (if available) */
  segments?: string[];

  /** Consent state */
  consent?: {
    gdprConsent?: boolean;
    ccpaOptOut?: boolean;
    coppaApplies?: boolean;
  };

  /** Previously shown creatives in this session */
  shownCreatives?: string[];

  /** Frequency cap state */
  frequencyCaps?: FrequencyCapState;
}

export interface FrequencyCapState {
  /** Creative ID -> count in current session */
  session: Record<string, number>;

  /** Advertiser ID -> count in current session */
  advertiserSession: Record<string, number>;

  /** Creative ID -> count in current hour */
  hourly?: Record<string, number>;

  /** Creative ID -> count in current day */
  daily?: Record<string, number>;
}

export interface FormatRequirements {
  /** Protocol */
  protocol: 'hls' | 'dash';

  /** Container */
  container: 'ts' | 'cmaf';

  /** Supported video codecs */
  videoCodecs: string[];

  /** Supported audio codecs */
  audioCodecs: string[];

  /** Resolution requirements */
  resolution?: {
    minWidth?: number;
    minHeight?: number;
    maxWidth?: number;
    maxHeight?: number;
  };

  /** DRM requirements */
  drm?: {
    required: boolean;
    schemes?: string[];
  };
}

export interface PodDecisionResponse {
  /** Pod ID */
  podId: string;

  /** Break ID */
  breakId: string;

  /** Decision for each slot */
  slotDecisions: SlotDecision[];

  /** Time taken for decision */
  decisionTimeMs: number;

  /** Whether we met the deadline */
  metDeadline: boolean;

  /** Any warnings */
  warnings: string[];
}

export interface SlotDecision {
  /** Slot ID */
  slotId: string;

  /** Filled or not */
  filled: boolean;

  /** Selected creative (if filled) */
  creative?: SelectedCreative;

  /** Slate (if not filled and allowed) */
  slate?: SlateInfo;

  /** Reason for decision */
  reason: string;

  /** Rejected candidates */
  rejectedCandidates?: RejectedCandidate[];
}

export interface SelectedCreative {
  /** Creative ID */
  creativeId: string;

  /** Advertiser ID */
  advertiserId: string;

  /** Campaign ID */
  campaignId?: string;

  /** Line item ID */
  lineItemId?: string;

  /** Partner that provided this creative */
  partnerId: string;

  /** Duration in seconds */
  duration: number;

  /** Media files */
  mediaFiles: MediaFile[];

  /** Tracking URLs */
  tracking?: {
    impression?: string[];
    start?: string[];
    firstQuartile?: string[];
    midpoint?: string[];
    thirdQuartile?: string[];
    complete?: string[];
    error?: string[];
  };

  /** Category for competitive separation */
  category?: string;

  /** Bid price (for reporting) */
  bidPrice?: number;
}

export interface MediaFile {
  /** URI of the media file */
  uri: string;

  /** Delivery type */
  delivery: 'streaming' | 'progressive';

  /** Container type */
  container: 'ts' | 'cmaf' | 'mp4' | 'webm';

  /** Codec string */
  codec?: string;

  /** Bitrate in kbps */
  bitrate?: number;

  /** Width in pixels */
  width?: number;

  /** Height in pixels */
  height?: number;

  /** Init segment URI (for CMAF) */
  initSegment?: string;
}

export interface SlateInfo {
  /** Slate asset ID */
  slateId: string;

  /** Media file for slate */
  mediaFile: MediaFile;

  /** Duration */
  duration: number;
}

export interface RejectedCandidate {
  /** Creative ID */
  creativeId: string;

  /** Partner ID */
  partnerId: string;

  /** Rejection reason */
  reason: RejectionReason;

  /** Additional details */
  details?: string;
}

export type RejectionReason =
  | 'timeout'
  | 'format_incompatible'
  | 'codec_mismatch'
  | 'duration_mismatch'
  | 'frequency_capped'
  | 'competitive_separation'
  | 'pacing_throttled'
  | 'circuit_open'
  | 'policy_violation'
  | 'error';

export interface DecisionDeadline {
  /** Absolute deadline timestamp */
  deadline: Date;

  /** Buffer time before hard deadline */
  bufferMs: number;

  /** Whether we're in safe mode */
  safeMode: boolean;

  /** Reduced partner set in safe mode */
  reducedPartners?: string[];
}

export interface DecisionSLAConfig {
  /** Maximum time for decision in ms */
  maxDecisionTimeMs: number;

  /** Partner timeout in ms */
  partnerTimeoutMs: number;

  /** Safe mode threshold (enable if latency exceeds) */
  safeModeThresholdMs: number;

  /** Maximum partners to query in safe mode */
  safeModeMaxPartners: number;

  /** Enable circuit breaker */
  circuitBreakerEnabled: boolean;

  /** Error rate threshold for circuit breaker */
  circuitBreakerErrorThreshold: number;

  /** Timeout rate threshold for circuit breaker */
  circuitBreakerTimeoutThreshold: number;

  /** Circuit breaker cooldown in ms */
  circuitBreakerCooldownMs: number;
}
