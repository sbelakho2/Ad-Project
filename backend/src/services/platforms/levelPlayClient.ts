/**
 * LevelPlay (Unity / ironSource) management client.
 *
 * Auth: publisher API key (`documented`, header name per LevelPlay's management API
 * docs; VERIFY-LIVE marker below). Reads waterfall instances (reusing the same read
 * used by Migration Studio imports) and edits waterfall entries and their prices.
 */
import { PlatformHttp } from './platformHttp';
import { schemaError, type WaterfallEntry } from './types';

export interface LevelPlayCredentials {
  apiKey: string;
  accountId: string;
}

export const LEVELPLAY_BASE_URL = process.env.LEVELPLAY_API_BASE_URL ?? 'https://api.ironsrc.com';

/**
 * Path templates for LevelPlay operations.
 * VERIFY-LIVE: the live-integration lane must confirm each template against the
 * publisher sandbox before autopilot writes are enabled for this platform.
 */
export const LEVELPLAY_ENDPOINTS = {
  listWaterfall: '/v1/adunits/{adUnitId}/waterfall',
  createEntry: '/v1/adunits/{adUnitId}/waterfall',
  updateEntryPrice: '/v1/adunits/{adUnitId}/waterfall/{entryId}',
  removeEntry: '/v1/adunits/{adUnitId}/waterfall/{entryId}',
} as const;

function isWaterfallList(data: unknown): data is { waterfall: Array<Record<string, unknown>> } {
  return !!data && typeof data === 'object' && Array.isArray((data as Record<string, unknown>).waterfall);
}

export class LevelPlayClient {
  private readonly http: PlatformHttp;

  constructor(credentials: LevelPlayCredentials, http?: PlatformHttp) {
    if (!credentials?.apiKey?.trim() || !credentials?.accountId?.trim()) {
      throw new Error('LevelPlay credentials require apiKey and accountId');
    }
    this.http = http ?? new PlatformHttp({
      platform: 'levelplay',
      baseUrl: LEVELPLAY_BASE_URL,
      auth: () => ({ 'Authorization': `Bearer ${credentials.apiKey}` }),
    });
  }

  async listWaterfall(adUnitId: string): Promise<WaterfallEntry[]> {
    const path = LEVELPLAY_ENDPOINTS.listWaterfall.replace('{adUnitId}', encodeURIComponent(adUnitId));
    const raw = await this.http.getValidated('listWaterfall', path, undefined, isWaterfallList);
    return raw.waterfall.map((item) => {
      const entryId = item.entry_id ?? item.id;
      const network = item.network;
      const instanceName = item.instance_name;
      const position = item.position;
      const priceEcpm = item.price_ecpm;
      if (typeof entryId !== 'string' || typeof network !== 'string' || typeof position !== 'number' || typeof priceEcpm !== 'number') {
        throw schemaError('levelplay', 'listWaterfall', `waterfall entry malformed: ${JSON.stringify(item)}`);
      }
      return {
        entryId,
        adUnitId,
        network,
        instanceName: typeof instanceName === 'string' ? instanceName : network,
        position,
        priceEcpm,
      };
    });
  }

  async updateEntryPrice(adUnitId: string, entryId: string, priceEcpm: number): Promise<WaterfallEntry> {
    if (!Number.isFinite(priceEcpm) || priceEcpm <= 0 || priceEcpm > 500) {
      throw schemaError('levelplay', 'updateEntryPrice', `price eCPM '${priceEcpm}' out of range (0, 500]`);
    }
    const path = LEVELPLAY_ENDPOINTS.updateEntryPrice
      .replace('{adUnitId}', encodeURIComponent(adUnitId))
      .replace('{entryId}', encodeURIComponent(entryId));
    const rounded = Math.round(priceEcpm * 100) / 100;
    await this.http.request({
      operation: 'updateEntryPrice',
      method: 'PATCH',
      path,
      body: { price_ecpm: rounded },
      idempotencyKey: `levelplay-price:${adUnitId}:${entryId}`,
    });
    return { entryId, adUnitId, network: 'unknown-until-refetch', instanceName: entryId, position: -1, priceEcpm: rounded };
  }

  async createEntry(adUnitId: string, network: string, instanceName: string, priceEcpm: number): Promise<{ entryId: string }> {
    if (!network.trim() || !instanceName.trim()) {
      throw schemaError('levelplay', 'createEntry', 'network and instanceName are required');
    }
    if (!Number.isFinite(priceEcpm) || priceEcpm <= 0 || priceEcpm > 500) {
      throw schemaError('levelplay', 'createEntry', `price eCPM '${priceEcpm}' out of range (0, 500]`);
    }
    const path = LEVELPLAY_ENDPOINTS.createEntry.replace('{adUnitId}', encodeURIComponent(adUnitId));
    const data = await this.http.request<unknown>({
      operation: 'createEntry',
      method: 'POST',
      path,
      body: { network, instance_name: instanceName, price_ecpm: Math.round(priceEcpm * 100) / 100 },
      idempotencyKey: `levelplay-create:${adUnitId}:${network}:${instanceName}`,
    });
    const entryId = (data as Record<string, unknown>)?.entry_id;
    if (typeof entryId !== 'string' || entryId === '') {
      throw schemaError('levelplay', 'createEntry', 'response missing entry_id');
    }
    return { entryId };
  }

  async removeEntry(adUnitId: string, entryId: string): Promise<void> {
    const path = LEVELPLAY_ENDPOINTS.removeEntry
      .replace('{adUnitId}', encodeURIComponent(adUnitId))
      .replace('{entryId}', encodeURIComponent(entryId));
    await this.http.request({
      operation: 'removeEntry',
      method: 'DELETE',
      path,
      idempotencyKey: `levelplay-remove:${adUnitId}:${entryId}`,
    });
  }
}
