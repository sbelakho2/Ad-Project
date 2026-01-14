import type { ExperimentArm, ExperimentMode } from '../../../types/migration';

export type AdFormat = 'banner' | 'interstitial' | 'rewarded' | 'native';

export interface AuctionContext {
  signal: AbortSignal;
  deadlineMs: number;
}

/**
 * Node-safe AbortError helpers (avoid relying on DOMException in Node.js)
 */
export const makeAbortError = (): Error => {
  const e: any = new Error('Aborted');
  e.name = 'AbortError';
  return e as Error;
};

export const isAbortError = (e: unknown): boolean => {
  return !!(e && typeof e === 'object' && (e as any).name === 'AbortError');
};

export interface AdapterBidRequest {
  requestId: string;
  publisherId: string;
  placementId: string;
  adFormat: AdFormat;
  floorCpm: number;
  consent?: {
    gdpr?: number; // 0/1
    gdprConsent?: string; // TCF v2
    usPrivacy?: string; // CCPA string
    coppa?: boolean;
  };
  device?: Record<string, unknown>;
  app?: Record<string, unknown>;
  user?: Record<string, unknown>;
  signal?: Record<string, unknown>;
  migration?: {
    experimentId: string;
    arm: ExperimentArm;
    assignmentTs: string;
    mirrorPercent?: number;
    mode?: ExperimentMode;
  };
}

/**
 * Lightweight runtime validation to protect adapters from malformed input.
 * Returns a normalized copy or throws an Error describing the first problem.
 */
export function validateAdapterBidRequest(req: AdapterBidRequest): AdapterBidRequest {
  const allowedFormats: AdFormat[] = ['banner', 'interstitial', 'rewarded', 'native'];
  if (!req || typeof req !== 'object') throw new Error('Invalid request');
  if (!req.requestId || typeof req.requestId !== 'string') throw new Error('requestId required');
  if (!req.publisherId || typeof req.publisherId !== 'string') throw new Error('publisherId required');
  if (!req.placementId || typeof req.placementId !== 'string') throw new Error('placementId required');
  if (!allowedFormats.includes(req.adFormat)) throw new Error('Unsupported adFormat');
  if (typeof req.floorCpm !== 'number' || !Number.isFinite(req.floorCpm) || req.floorCpm < 0) {
    throw new Error('floorCpm must be a non-negative finite number');
  }
  if (req.consent && typeof req.consent !== 'object') throw new Error('consent must be an object');
  return req;
}

export interface AdapterBid {
  adapter: string;
  cpm: number;
  currency: 'USD';
  creativeUrl: string;
  ttlSeconds: number;
  meta?: Record<string, unknown>;
  latencyMs?: number;
}

export type NoBid = { nobid: true; reason?: string };

export interface AdapterDefinition {
  name: string;
  supports: AdFormat[];
  timeoutMs: number; // per-adapter ceiling
  requestBid: (ctx: AuctionContext, req: AdapterBidRequest) => Promise<AdapterBid | NoBid>;
}
