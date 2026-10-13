/**
 * AppLovin MAX management client.
 *
 * Auth: publisher API key (MAX -> Account -> API keys), sent as the `X-API-Key`
 * header (`documented`). Base URL is env-overridable for the sandbox; the live
 * endpoint map carries per-operation verification markers (see types.ts policy).
 */
import { PlatformHttp } from './platformHttp';
import {
  schemaError,
  type AdUnit,
  type CountryFloor,
  type ExperimentResult,
  type ExperimentSpec,
} from './types';

export interface MaxCredentials {
  apiKey: string;
  accountId: string;
}

export const MAX_BASE_URL = process.env.MAX_API_BASE_URL ?? 'https://api.applovin.com';

/** Path templates for every operation the autopilot may perform on MAX. */
export const MAX_ENDPOINTS = {
  listAdUnits: '/v1/max/ad_units',
  getAdUnit: '/v1/max/ad_units/{adUnitId}',
  setCountryFloor: '/v1/max/ad_units/{adUnitId}/floors/{country}',
  createExperiment: '/v1/max/experiments',
  getExperiment: '/v1/max/experiments/{experimentId}',
} as const;

const COUNTRY_RE = /^[A-Z]{2}$/;

export function normalizeCountry(country: string): string {
  const up = country.trim().toUpperCase();
  if (!COUNTRY_RE.test(up)) {
    throw schemaError('applovin_max', 'setCountryFloor', `invalid country code '${country}' (expected ISO 3166-1 alpha-2)`);
  }
  return up;
}

export function assertValidFloor(floorEcpm: number): number {
  if (!Number.isFinite(floorEcpm) || floorEcpm < 0 || floorEcpm > 500) {
    throw schemaError('applovin_max', 'setCountryFloor', `floor eCPM '${floorEcpm}' out of range [0, 500]`);
  }
  return Math.round(floorEcpm * 100) / 100;
}

function isAdUnitList(data: unknown): data is { ad_units: Array<Record<string, unknown>> } {
  return !!data && typeof data === 'object' && Array.isArray((data as Record<string, unknown>).ad_units);
}

export class MaxClient {
  private readonly http: PlatformHttp;

  constructor(credentials: MaxCredentials, http?: PlatformHttp) {
    if (!credentials?.apiKey?.trim() || !credentials?.accountId?.trim()) {
      throw new Error('MAX credentials require apiKey and accountId');
    }
    this.http = http ?? new PlatformHttp({
      platform: 'applovin_max',
      baseUrl: MAX_BASE_URL,
      auth: () => ({ 'X-API-Key': credentials.apiKey }),
    });
  }

  async listAdUnits(): Promise<AdUnit[]> {
    const raw = await this.http.getValidated('listAdUnits', MAX_ENDPOINTS.listAdUnits, undefined, isAdUnitList);
    const units: AdUnit[] = [];
    for (const item of raw.ad_units) {
      const id = item.ad_unit_id ?? item.id;
      const name = item.name;
      const format = item.format;
      if (typeof id !== 'string' || typeof name !== 'string' || typeof format !== 'string') {
        throw schemaError('applovin_max', 'listAdUnits', `ad unit entry missing required fields: ${JSON.stringify(item)}`);
      }
      units.push({
        platformUnitId: id,
        name,
        format: format as AdUnit['format'],
        enabled: item.enabled !== false,
      });
    }
    return units;
  }

  async setCountryFloor(adUnitId: string, country: string, floorEcpm: number): Promise<CountryFloor> {
    const normalized = normalizeCountry(country);
    const floor = assertValidFloor(floorEcpm);
    const path = MAX_ENDPOINTS.setCountryFloor.replace('{adUnitId}', encodeURIComponent(adUnitId)).replace('{country}', normalized);
    await this.http.request({
      operation: 'setCountryFloor',
      method: 'PUT',
      path,
      body: { floor_ecpm: floor },
    });
    return { country: normalized, floorEcpm: floor };
  }

  async createExperiment(spec: ExperimentSpec): Promise<{ experimentId: string }> {
    if (spec.variants < 2 || spec.variants > 4) {
      throw schemaError('applovin_max', 'createExperiment', `variants must be 2..4, got ${spec.variants}`);
    }
    if (!spec.changeDescription.trim()) {
      throw schemaError('applovin_max', 'createExperiment', 'changeDescription is required (auditable single change)');
    }
    const body = {
      name: spec.name,
      ad_unit_id: spec.adUnitId,
      metric: spec.metric,
      variant_count: spec.variants,
      change_description: spec.changeDescription,
    };
    const data = await this.http.request<unknown>({
      operation: 'createExperiment',
      method: 'POST',
      path: MAX_ENDPOINTS.createExperiment,
      body,
      idempotencyKey: `max-experiment:${spec.name}:${spec.adUnitId}`,
    });
    const experimentId = (data as Record<string, unknown>)?.experiment_id;
    if (typeof experimentId !== 'string' || experimentId === '') {
      throw schemaError('applovin_max', 'createExperiment', 'response missing experiment_id');
    }
    return { experimentId };
  }

  async getExperiment(experimentId: string): Promise<ExperimentResult> {
    const path = MAX_ENDPOINTS.getExperiment.replace('{experimentId}', encodeURIComponent(experimentId));
    const data = await this.http.request<unknown>({ operation: 'getExperiment', method: 'GET', path });
    const d = data as Record<string, unknown>;
    if (!d || typeof d !== 'object' || typeof d.state !== 'string' || !Array.isArray(d.variants)) {
      throw schemaError('applovin_max', 'getExperiment', 'response missing state/variants');
    }
    const results = (d.variants as Array<Record<string, unknown>>).map((v) => {
      const variantId = v.variant_id;
      const impressions = v.impressions;
      const revenue = v.revenue;
      if (typeof variantId !== 'string' || typeof impressions !== 'number' || typeof revenue !== 'number') {
        throw schemaError('applovin_max', 'getExperiment', `variant entry malformed: ${JSON.stringify(v)}`);
      }
      const ecpm = impressions > 0 ? (revenue / impressions) * 1000 : 0;
      return { variantId, impressions, revenue, ecpm };
    });
    return {
      experimentId,
      state: d.state === 'completed' || d.state === 'stopped' ? d.state : 'running',
      results,
      liftPercent: typeof d.lift_percent === 'number' ? d.lift_percent : undefined,
      confidence: typeof d.confidence === 'number' ? d.confidence : undefined,
      winnerVariantId: typeof d.winner_variant_id === 'string' ? d.winner_variant_id : undefined,
    };
  }
}
