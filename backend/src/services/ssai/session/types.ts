/**
 * SSAI Session Types
 *
 * Defines all types related to SSAI session management, including
 * session identifiers, configuration, and state.
 */

/**
 * Device capability profile for session scoping
 */
export interface DeviceCapabilityProfile {
  /** Device type identifier */
  deviceType: 'web' | 'ctv' | 'mobile' | 'stb';

  /** Supported protocol */
  protocol: 'hls' | 'dash';

  /** Supported container format */
  container: 'ts' | 'cmaf' | 'fmp4';

  /** Maximum supported resolution */
  maxResolution: '480p' | '720p' | '1080p' | '4k';

  /** Supported codecs */
  codecs: {
    video: string[]; // e.g., ['avc1', 'hevc', 'vp9', 'av1']
    audio: string[]; // e.g., ['aac', 'ac3', 'eac3', 'opus']
  };

  /** DRM support */
  drm?: {
    supported: boolean;
    systems: string[]; // e.g., ['widevine', 'fairplay', 'playready']
  };

  /** HDR support */
  hdr?: {
    supported: boolean;
    types: string[]; // e.g., ['hdr10', 'dolby-vision', 'hlg']
  };

  /** Caption support */
  captions?: {
    supported: boolean;
    formats: string[]; // e.g., ['webvtt', 'cea-608', 'cea-708']
  };
}

/**
 * Consent and policy snapshot for session scoping
 */
export interface ConsentPolicySnapshot {
  /** GDPR applies flag */
  gdprApplies: boolean;

  /** TCF consent string */
  tcfConsent?: string;

  /** USP (CCPA) privacy string */
  usPrivacy?: string;

  /** GPP consent string */
  gpp?: string;

  /** COPPA flag */
  coppa: boolean;

  /** Timestamp when consent was captured */
  capturedAt: string;

  /** Allowed targeting */
  targeting: {
    personalized: boolean;
    contextual: boolean;
    crossDevice: boolean;
  };
}

/**
 * Session configuration options
 */
export interface SSAISessionConfig {
  /** Tenant identifier */
  tenantId: string;

  /** Channel or asset identifier */
  channelId: string;

  /** Asset ID for VOD */
  assetId?: string;

  /** Stream type */
  streamType: 'live' | 'vod' | 'event';

  /** Device capability profile */
  deviceProfile: DeviceCapabilityProfile;

  /** Consent and policy snapshot */
  consent: ConsentPolicySnapshot;

  /** Session TTL in seconds */
  ttlSeconds?: number;

  /** Custom metadata */
  metadata?: Record<string, unknown>;
}

/**
 * Session state
 */
export type SSAISessionState = 'active' | 'expired' | 'terminated';

/**
 * SSAI Session
 */
export interface SSAISession {
  /** Unique session identifier */
  sessionId: string;

  /** Session configuration */
  config: SSAISessionConfig;

  /** Session state */
  state: SSAISessionState;

  /** Creation timestamp */
  createdAt: string;

  /** Last access timestamp */
  lastAccessedAt: string;

  /** Expiration timestamp */
  expiresAt: string;

  /** Number of manifest requests */
  manifestRequests: number;

  /** Current manifest version */
  manifestVersion: number;

  /** Active pods in this session */
  activePods: string[];

  /** Completed pods in this session */
  completedPods: string[];

  /** Session token for authentication */
  token: string;

  /** Token expiration */
  tokenExpiresAt: string;
}

/**
 * Session creation request
 */
export interface CreateSessionRequest {
  /** Session configuration */
  config: SSAISessionConfig;

  /** Client IP for rate limiting */
  clientIp?: string;

  /** User agent for device detection */
  userAgent?: string;

  /** Idempotency key to prevent session storms */
  idempotencyKey?: string;
}

/**
 * Session creation response
 */
export interface CreateSessionResponse {
  /** Whether session was created or retrieved (idempotent) */
  created: boolean;

  /** The session */
  session: SSAISession;

  /** Manifest URLs */
  manifestUrls: {
    master: string;
    media?: string[];
  };
}

/**
 * Session refresh request
 */
export interface RefreshSessionRequest {
  /** Session ID */
  sessionId: string;

  /** Session token */
  token: string;
}

/**
 * Session lookup options
 */
export interface SessionLookupOptions {
  /** Include expired sessions */
  includeExpired?: boolean;

  /** Validate token */
  validateToken?: boolean;
}

/**
 * Rate limit configuration
 */
export interface SessionRateLimitConfig {
  /** Max sessions per tenant per minute */
  maxSessionsPerTenantPerMinute: number;

  /** Max sessions per IP per minute */
  maxSessionsPerIpPerMinute: number;

  /** Max manifest requests per session per second */
  maxManifestRequestsPerSecond: number;

  /** Retry storm threshold (requests in window) */
  retryStormThreshold: number;

  /** Retry storm window in milliseconds */
  retryStormWindowMs: number;
}

/**
 * Default rate limit configuration
 */
export const DEFAULT_RATE_LIMIT_CONFIG: SessionRateLimitConfig = {
  maxSessionsPerTenantPerMinute: 10000,
  maxSessionsPerIpPerMinute: 100,
  maxManifestRequestsPerSecond: 10,
  retryStormThreshold: 50,
  retryStormWindowMs: 10000,
};

/**
 * Session TTL configuration
 */
export interface SessionTTLConfig {
  /** Live session TTL in seconds (with refresh) */
  liveTtlSeconds: number;

  /** VOD session TTL in seconds */
  vodTtlSeconds: number;

  /** Event session TTL in seconds */
  eventTtlSeconds: number;

  /** Token TTL in seconds */
  tokenTtlSeconds: number;

  /** Refresh threshold (refresh when TTL below this) */
  refreshThresholdSeconds: number;
}

/**
 * Default TTL configuration
 */
export const DEFAULT_TTL_CONFIG: SessionTTLConfig = {
  liveTtlSeconds: 3600, // 1 hour
  vodTtlSeconds: 86400, // 24 hours
  eventTtlSeconds: 7200, // 2 hours
  tokenTtlSeconds: 900, // 15 minutes
  refreshThresholdSeconds: 300, // 5 minutes
};

/**
 * Session metrics
 */
export interface SessionMetrics {
  /** Total active sessions */
  activeSessions: number;

  /** Sessions by tenant */
  sessionsByTenant: Record<string, number>;

  /** Sessions by stream type */
  sessionsByStreamType: Record<string, number>;

  /** Sessions created in last minute */
  sessionsCreatedLastMinute: number;

  /** Session creation rate */
  sessionCreationRate: number;

  /** Average session duration */
  avgSessionDurationSeconds: number;

  /** Rate limit rejections */
  rateLimitRejections: number;
}
