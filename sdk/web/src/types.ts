export type ConsentState = {
  gdprApplies?: boolean;
  tcfConsent?: string | null; // IAB TCF v2 string
  usPrivacy?: string | null; // CCPA/USP string
  gpp?: string | null; // IAB GPP string
  coppa?: boolean;
};

export type InitOptions = {
  endpoint: string; // e.g., https://api.example.com
  publisherId?: string;
  appId?: string;
  timeoutMs?: number; // default 2000ms
  // Reliability controls (FIX-05)
  maxRetries?: number; // default 2 for retryable errors (network/5xx)
  retryBackoffBaseMs?: number; // default 100ms (exponential backoff base)
  retryJitterMs?: number; // default +/-50ms jitter
  sdkVersion?: string;
  debug?: boolean; // enable verbose SDK logs (no PII)
  telemetryEnabled?: boolean; // allow emitting lightweight telemetry events
  // Distributed tracing
  correlationIdGenerator?: () => string; // Custom correlation ID generator
};

export type AdRequest = {
  placement: string;
  adType: 'banner' | 'interstitial' | 'rewarded' | 'rewarded_interstitial' | 'native' | 'app_open';
  width?: number;
  height?: number;
  testMode?: boolean;
  extras?: Record<string, unknown>;
};

export type AdCreative = {
  id: string;
  html?: string; // for banner/web content
  vastTagUrl?: string; // for video
  tracking?: Record<string, string>;
};

export type AdResponse = {
  requestId: string;
  fill: boolean;
  price?: number;
  currency?: string;
  creative?: AdCreative | null;
  ttlSeconds?: number;
  // Additional fields for parity with native SDKs
  networkName?: string; // Winning ad network
  adType?: 'banner' | 'interstitial' | 'rewarded' | 'rewarded_interstitial' | 'native' | 'app_open';
  placementId?: string;
  expiryTimeMs?: number; // Absolute expiry timestamp
};

export type SdkEvent =
  | 'sdk:initialized'
  | 'consent:updated'
  | 'ad:requested'
  | 'ad:filled'
  | 'ad:error';
