/**
 * Platform management-API layer (new business model core).
 *
 * Apex sits ON TOP of the publisher's existing mediation (AppLovin MAX, LevelPlay,
 * AdMob). This layer is the only place that talks to each platform's management API:
 * it reads setup/revenue and writes optimizations using the publisher's own key.
 * No ads flow through Apex; a platform outage never affects serving.
 *
 * CONTRACT POLICY (adversarial dogfooding law):
 * - Every operation's request/response shape is pinned by offline contract tests with
 *   recorded fixtures (backend/tests/platforms).
 * - Endpoint paths that are not verifiable from public platform documentation in this
 *   repo carry a `VERIFY-LIVE` marker: the live-integration lane must confirm them
 *   against the platform sandbox before an operation is enabled for autopilot writes.
 */

export type PlatformId = 'applovin_max' | 'levelplay' | 'admob';

export type SourceVerification = 'documented' | 'verify-live';

export interface PlatformOperation<Args extends unknown[], Result> {
  readonly name: string;
  readonly method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  readonly verification: SourceVerification;
  readonly idempotent: boolean;
  readonly invoke: (...args: Args) => Promise<Result>;
}

export interface AdUnit {
  platformUnitId: string;
  name: string;
  format: 'banner' | 'interstitial' | 'rewarded' | 'native' | 'app_open';
  enabled: boolean;
  countryFloors?: CountryFloor[];
}

export interface CountryFloor {
  country: string;
  floorEcpm: number;
}

export interface WaterfallEntry {
  entryId: string;
  adUnitId: string;
  network: string;
  instanceName: string;
  position: number;
  priceEcpm: number;
}

export interface MediationGroupMapping {
  adUnitId: string;
  network: string;
  ecpm: number;
}

export interface MediationGroup {
  mediationGroupId: string;
  name: string;
  format: AdUnit['format'];
  mappings: MediationGroupMapping[];
}

export interface ExperimentSpec {
  name: string;
  adUnitId: string;
  metric: 'ecpm' | 'revenue' | 'arpdau';
  variants: number;
  /** Human-auditable description of the single change under test. */
  changeDescription: string;
}

export interface ExperimentVariantResult {
  variantId: string;
  impressions: number;
  revenue: number;
  ecpm: number;
}

export interface ExperimentResult {
  experimentId: string;
  state: 'running' | 'stopped' | 'completed';
  results: ExperimentVariantResult[];
  /** Percent lift of best variant vs control as measured by the PLATFORM. */
  liftPercent?: number;
  confidence?: number;
  winnerVariantId?: string;
}

export class PlatformApiError extends Error {
  constructor(
    message: string,
    readonly platform: PlatformId,
    readonly operation: string,
    readonly options: {
      status?: number;
      code?: string;
      retryable: boolean;
      /** Capped Retry-After hint (ms) when the platform provided one. */
      retryAfterMs?: number;
      cause?: unknown;
    },
  ) {
    super(message);
    this.name = 'PlatformApiError';
    this.retryable = options.retryable;
    this.status = options.status;
    this.code = options.code;
    this.retryAfterMs = options.retryAfterMs;
    if (options.cause !== undefined) this.cause = options.cause;
  }

  readonly retryable: boolean;
  readonly status?: number;
  readonly code?: string;
  readonly retryAfterMs?: number;
  readonly cause?: unknown;
}

export function isRetryableStatus(status: number | undefined): boolean {
  return status === 429 || (status !== undefined && status >= 500 && status < 600);
}

/** Fail-closed placeholder for malformed platform responses. */
export function schemaError(platform: PlatformId, operation: string, detail: string): PlatformApiError {
  return new PlatformApiError(`Malformed ${platform} response for ${operation}: ${detail}`, platform, operation, {
    code: 'SCHEMA_INVALID',
    retryable: false,
  });
}
