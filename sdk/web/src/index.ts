import { emitter } from './events';
import { Errors, SdkError } from './errors';
import { requestAuction, type AuctionClientConfig } from './auctionClient';
import type { ConsentState, InitOptions, AdRequest, AdResponse, SdkEvent } from './types';
import { ConsentSchema, AdRequestSchema } from './schemas';
import { SUPPORTED_NETWORKS, getSupportedAdapters } from './adapters';
import {
  computeConfigHash,
  validateConfigHash,
  fetchConfigWithHash,
  createDefaultConfig,
  type SDKConfig,
} from './configHash';

type State = {
  initialized: boolean;
  consent?: ConsentState;
  cfg?: AuctionClientConfig;
  debug?: boolean;
  telemetryEnabled?: boolean;
};

const state: State = { initialized: false };

export function init(options: InitOptions) {
  const endpoint = options?.endpoint?.trim();
  if (!endpoint) throw Errors.invalidOptions('endpoint is required');
  const timeoutMs = Math.max(1, options.timeoutMs ?? 2000);
  const maxRetries = Math.max(0, options.maxRetries ?? 2);
  const retryBackoffBaseMs = Math.max(0, options.retryBackoffBaseMs ?? 100);
  const retryJitterMs = Math.max(0, options.retryJitterMs ?? 50);

  state.cfg = {
    endpoint,
    timeoutMs,
    publisherId: options.publisherId,
    appId: options.appId,
    sdkVersion: options.sdkVersion ?? '0.1.0',
    maxRetries,
    retryBackoffBaseMs,
    retryJitterMs,
  };
  state.initialized = true;
  state.debug = Boolean(options.debug);
  state.telemetryEnabled = options.telemetryEnabled !== false; // default true
}

export function setConsent(consent: ConsentState) {
  const parsed = ConsentSchema.safeParse(consent);
  if (!parsed.success) throw Errors.validation('Invalid consent object', { issues: parsed.error.issues });
  state.consent = parsed.data;
  emitter.emit('consent:updated', { consent: state.consent });
}

export async function requestAd(req: AdRequest): Promise<AdResponse> {
  if (!state.initialized || !state.cfg) throw Errors.initRequired();
  const parsed = AdRequestSchema.safeParse(req);
  if (!parsed.success) throw Errors.validation('Invalid ad request', { issues: parsed.error.issues });
  emitter.emit('ad:requested', { request: parsed.data });
  try {
    const resp = await requestAuction(state.cfg, state.consent, parsed.data);
    emitter.emit('ad:filled', { response: resp });
    return resp;
  } catch (e) {
    const err = e as SdkError;
    emitter.emit('ad:error', { error: { code: (err as any).code, message: err.message } });
    throw e;
  }
}

export function on(event: SdkEvent, handler: (payload?: any) => void) {
  return emitter.on(event, handler);
}

export { Errors, SdkError };
export type { ConsentState, InitOptions, AdRequest, AdResponse, SdkEvent };
export { SUPPORTED_NETWORKS, getSupportedAdapters };
export {
  computeConfigHash,
  validateConfigHash,
  fetchConfigWithHash,
  createDefaultConfig,
  type SDKConfig,
};

// SSAI (Server-Side Ad Insertion) support
export * as ssai from './ssai';
