/**
 * SSAI Types for Web SDK
 *
 * Types and interfaces for Server-Side Ad Insertion integration.
 */

/**
 * SSAI Session configuration
 */
export interface SSAIConfig {
  /** SSAI service endpoint */
  endpoint: string;

  /** Tenant ID */
  tenantId: string;

  /** Content stream URL (origin) */
  contentUrl: string;

  /** Content format */
  format: 'hls' | 'dash';

  /** Device capabilities */
  capabilities?: DeviceCapabilities;

  /** Consent data */
  consent?: SSAIConsentData;

  /** Session timeout (seconds) */
  timeoutSeconds?: number;

  /** Enable debug mode */
  debug?: boolean;
}

/**
 * Device capabilities for ad selection
 */
export interface DeviceCapabilities {
  /** Max supported resolution */
  maxResolution?: {
    width: number;
    height: number;
  };

  /** Supported codecs */
  codecs?: string[];

  /** Supports HDR */
  supportsHdr?: boolean;

  /** Supports Dolby */
  supportsDolby?: boolean;

  /** Max bitrate (bps) */
  maxBitrate?: number;

  /** Device type */
  deviceType?: 'desktop' | 'mobile' | 'tablet' | 'ctv' | 'stb';
}

/**
 * Consent data for SSAI
 */
export interface SSAIConsentData {
  /** GDPR applies */
  gdprApplies?: boolean;

  /** TCF consent string */
  tcfConsent?: string;

  /** US Privacy string (CCPA) */
  usPrivacy?: string;

  /** GPP string */
  gpp?: string;

  /** COPPA applies */
  coppa?: boolean;
}

/**
 * SSAI Session response
 */
export interface SSAISession {
  /** Session ID */
  sessionId: string;

  /** Manifest URL (with session token) */
  manifestUrl: string;

  /** Session token for tracking */
  sessionToken: string;

  /** Tracking base URL */
  trackingUrl: string;

  /** Session expires at */
  expiresAt: Date;

  /** Original content URL */
  contentUrl: string;
}

/**
 * SSAI Tracking event types
 */
export type SSAITrackingEventType =
  | 'impression'
  | 'start'
  | 'firstQuartile'
  | 'midpoint'
  | 'thirdQuartile'
  | 'complete'
  | 'error'
  | 'skip'
  | 'mute'
  | 'unmute'
  | 'pause'
  | 'resume'
  | 'fullscreen'
  | 'exitFullscreen'
  | 'click';

/**
 * SSAI Tracking event
 */
export interface SSAITrackingEvent {
  /** Event type */
  type: SSAITrackingEventType;

  /** Pod ID */
  podId: string;

  /** Slot ID */
  slotId: string;

  /** Creative ID */
  creativeId: string;

  /** Current media time (seconds) */
  mediaTime: number;

  /** Timestamp */
  timestamp: Date;

  /** Additional metadata */
  metadata?: Record<string, unknown>;
}

/**
 * SSAI Ad break (pod) info
 */
export interface SSAIAdBreak {
  /** Pod ID */
  podId: string;

  /** Break position (seconds from start) */
  position: number;

  /** Break duration (seconds) */
  duration: number;

  /** Number of ads */
  adCount: number;

  /** Break type */
  type: 'preroll' | 'midroll' | 'postroll';

  /** Individual ads */
  ads: SSAIAd[];
}

/**
 * SSAI Ad info
 */
export interface SSAIAd {
  /** Slot ID */
  slotId: string;

  /** Creative ID */
  creativeId: string;

  /** Ad duration (seconds) */
  duration: number;

  /** Advertiser name */
  advertiser?: string;

  /** Ad title */
  title?: string;

  /** Skip offset (seconds, if skippable) */
  skipOffset?: number;

  /** Click-through URL */
  clickThroughUrl?: string;
}

/**
 * SSAI player events
 */
export interface SSAIPlayerEvents {
  /** Called when entering an ad break */
  onAdBreakStart?: (adBreak: SSAIAdBreak) => void;

  /** Called when exiting an ad break */
  onAdBreakEnd?: (adBreak: SSAIAdBreak) => void;

  /** Called when an ad starts */
  onAdStart?: (ad: SSAIAd, adBreak: SSAIAdBreak) => void;

  /** Called when an ad ends */
  onAdEnd?: (ad: SSAIAd, adBreak: SSAIAdBreak) => void;

  /** Called on ad progress */
  onAdProgress?: (ad: SSAIAd, progress: number) => void;

  /** Called on ad error */
  onAdError?: (error: SSAIError) => void;

  /** Called when ad is skipped */
  onAdSkipped?: (ad: SSAIAd) => void;

  /** Called when ad is clicked */
  onAdClicked?: (ad: SSAIAd) => void;
}

/**
 * SSAI Error
 */
export interface SSAIError {
  /** Error code */
  code: string;

  /** Error message */
  message: string;

  /** Is fatal */
  fatal: boolean;

  /** Related pod ID */
  podId?: string;

  /** Related slot ID */
  slotId?: string;
}

/**
 * SSAI session state
 */
export type SSAISessionState =
  | 'idle'
  | 'initializing'
  | 'active'
  | 'inAdBreak'
  | 'error'
  | 'expired';

/**
 * SSAI Metrics
 */
export interface SSAIMetrics {
  /** Total ad breaks seen */
  adBreakCount: number;

  /** Total ads seen */
  adCount: number;

  /** Total ad time (seconds) */
  totalAdTime: number;

  /** Completed ads */
  completedAds: number;

  /** Skipped ads */
  skippedAds: number;

  /** Errored ads */
  erroredAds: number;

  /** Fill rate (0-1) */
  fillRate: number;
}
