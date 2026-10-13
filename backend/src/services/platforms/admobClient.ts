/**
 * AdMob management client.
 *
 * Auth: OAuth2 bearer token (AdMob Management API uses Google OAuth2 scopes, not a
 * plain API key — `documented`). Ad unit mappings fetch eCPM values; the autopilot
 * writes updated eCPM values for mediation group mappings.
 *
 * AdMob's own Experiments surface in the management API is not fully documented for
 * third parties; experiment read/write operations carry VERIFY-LIVE markers and are
 * disabled for autopilot writes until the live-integration lane confirms them.
 */
import { PlatformHttp } from './platformHttp';
import { schemaError, type MediationGroup, type MediationGroupMapping } from './types';

export interface AdmobCredentials {
  accessToken: string;
  accountId: string; // e.g. "accounts/pub-1234567890123456"
}

export const ADMOB_BASE_URL = process.env.ADMOB_API_BASE_URL ?? 'https://admob.googleapis.com';

export const ADMOB_ENDPOINTS = {
  listAdUnits: '/v1/{accountId}/adUnits',
  listMediationGroups: '/v1/{accountId}/mediationGroups',
  getMediationGroup: '/v1/{accountId}/mediationGroups/{mediationGroupId}',
  /** VERIFY-LIVE: batch update of mediation group mappings (eCPM / ad unit targets). */
  updateMediationGroup: '/v1/{accountId}/mediationGroups/{mediationGroupId}',
} as const;

export function normalizeAccountId(accountId: string): string {
  const trimmed = accountId.trim();
  const withPrefix = trimmed.startsWith('accounts/') ? trimmed : `accounts/${trimmed}`;
  if (!/^accounts\/pub-\d{6,}$/.test(withPrefix)) {
    throw schemaError('admob', 'accountId', `invalid AdMob account id '${accountId}' (expected accounts/pub-<digits>)`);
  }
  return withPrefix;
}

function assertEcpm(ecpm: number): number {
  if (!Number.isFinite(ecpm) || ecpm <= 0 || ecpm > 500) {
    throw schemaError('admob', 'updateMediationGroup', `eCPM '${ecpm}' out of range (0, 500]`);
  }
  return Math.round(ecpm * 100) / 100;
}

export class AdmobClient {
  private readonly http: PlatformHttp;
  private readonly accountId: string;

  constructor(credentials: AdmobCredentials, http?: PlatformHttp) {
    if (!credentials?.accessToken?.trim() || !credentials?.accountId?.trim()) {
      throw new Error('AdMob credentials require accessToken and accountId');
    }
    this.accountId = normalizeAccountId(credentials.accountId);
    this.http = http ?? new PlatformHttp({
      platform: 'admob',
      baseUrl: ADMOB_BASE_URL,
      auth: () => ({ 'Authorization': `Bearer ${credentials.accessToken}` }),
    });
  }

  /** GET list and follow nextPageToken pages (bounded, fail-closed on shape drift). */
  private async listAll<T>(operation: string, path: string, key: string, mapItem: (item: Record<string, unknown>) => T): Promise<T[]> {
    const out: T[] = [];
    let pageToken: string | undefined;
    for (let page = 0; page < 50; page++) {
      const params = pageToken ? { pageToken } : undefined;
      const data = await this.http.request<unknown>({ operation, method: 'GET', path, params });
      const d = data as Record<string, unknown>;
      if (!d || typeof d !== 'object' || !Array.isArray(d[key])) {
        throw schemaError('admob', operation, `response missing array '${key}'`);
      }
      for (const item of d[key] as Array<Record<string, unknown>>) out.push(mapItem(item));
      const next = d.nextPageToken;
      if (typeof next !== 'string' || next === '') return out;
      pageToken = next;
    }
    throw schemaError('admob', operation, 'pagination exceeded 50 pages (runaway nextPageToken)');
  }

  async listMediationGroups(): Promise<MediationGroup[]> {
    const path = ADMOB_ENDPOINTS.listMediationGroups.replace('{accountId}', this.accountId);
    return this.listAll('listMediationGroups', path, 'mediationGroups', (item) => {
      const id = item.mediationGroupId;
      const name = item.name;
      if (typeof id !== 'string' || typeof name !== 'string' || !Array.isArray(item.adUnitMappings)) {
        throw schemaError('admob', 'listMediationGroups', `mediation group malformed: ${JSON.stringify(item)}`);
      }
      const mappings = (item.adUnitMappings as Array<Record<string, unknown>>).map((m): MediationGroupMapping => {
        const adUnitId = m.adUnitId;
        const network = m.network;
        const ecpm = typeof m.ecpm === 'number' ? m.ecpm : Number(m.eCPM ?? 0);
        if (typeof adUnitId !== 'string' || typeof network !== 'string' || !Number.isFinite(ecpm)) {
          throw schemaError('admob', 'listMediationGroups', `mapping malformed: ${JSON.stringify(m)}`);
        }
        return { adUnitId, network, ecpm };
      });
      return {
        mediationGroupId: id,
        name,
        format: (typeof item.format === 'string' ? item.format : 'banner') as MediationGroup['format'],
        mappings,
      };
    });
  }

  /**
   * Update the eCPM of one mapping within a mediation group.
   * Read-modify-write: the current group is re-fetched, the single mapping is changed,
   * and the full mapping list is written back (platform API replaces the whole list).
   */
  async updateMappingEcpm(mediationGroupId: string, adUnitId: string, ecpm: number): Promise<MediationGroupMapping> {
    const newEcpm = assertEcpm(ecpm);
    const groups = await this.listMediationGroups();
    const group = groups.find((g) => g.mediationGroupId === mediationGroupId);
    if (!group) {
      throw schemaError('admob', 'updateMediationGroup', `mediation group '${mediationGroupId}' not found`);
    }
    const target = group.mappings.find((m) => m.adUnitId === adUnitId);
    if (!target) {
      throw schemaError('admob', 'updateMediationGroup', `ad unit '${adUnitId}' not in group '${mediationGroupId}'`);
    }
    const updated = group.mappings.map((m) => (m.adUnitId === adUnitId ? { ...m, ecpm: newEcpm } : m));
    const path = ADMOB_ENDPOINTS.updateMediationGroup
      .replace('{accountId}', this.accountId)
      .replace('{mediationGroupId}', encodeURIComponent(mediationGroupId));
    await this.http.request({
      operation: 'updateMediationGroup',
      method: 'PATCH',
      path,
      body: { adUnitMappings: updated },
      idempotencyKey: `admob-ecpm:${mediationGroupId}:${adUnitId}:${newEcpm}`,
    });
    return { adUnitId, network: target.network, ecpm: newEcpm };
  }
}
