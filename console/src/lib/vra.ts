/**
 * VRA client for Console (read-only)
 * Mirrors backend routes under /api/v1/recon/* and /api/v1/proofs/*.
 * Additive and safe: only GET/POST with feature-flag guarded endpoints.
 */

export type ReconOverviewItem = {
  network: string;
  format: string;
  country: string;
  impressions: number;
  paid: number;
  expected?: number;
};

export type ReconOverviewResult = {
  coveragePercent: number;
  variancePercent: number;
  totals: {
    impressions: number;
    paid: number;
    expected: number;
  };
  byBreakdown: ReconOverviewItem[];
  byNetwork?: Array<{
    network: string;
    impressions: number;
    paid: number;
    expected: number;
  }>;
};

export type ReconDeltaItem = {
  kind: 'underpay' | 'missing' | 'viewability_gap' | 'ivt_outlier' | 'fx_mismatch' | 'timing_lag';
  amount: number;
  currency: string;
  reasonCode: string;
  windowStart: string;
  windowEnd: string;
  evidenceId: string;
  confidence: number;
};

export type ReconDeltaQuery = {
  app_id?: string;
  from?: string;
  to?: string;
  kind?: ReconDeltaItem['kind'];
  min_conf?: number;
  page?: number;
  page_size?: number;
};

export async function getOverview(params: { app_id?: string; from?: string; to?: string }): Promise<ReconOverviewResult> {
  const url = new URL('/api/v1/recon/overview', window.location.origin);
  if (params.app_id) url.searchParams.set('app_id', params.app_id);
  if (params.from) url.searchParams.set('from', params.from);
  if (params.to) url.searchParams.set('to', params.to);
  const res = await fetch(url.toString(), { credentials: 'include' });
  if (!res.ok) throw new Error(`Overview failed: ${res.status}`);
  const body = await res.json();
  return body.data as ReconOverviewResult;
}

export async function getDeltas(query: ReconDeltaQuery): Promise<{ items: ReconDeltaItem[]; page: number; pageSize: number; total: number }> {
  const url = new URL('/api/v1/recon/deltas', window.location.origin);
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null) continue;
    url.searchParams.set(k, String(v));
  }
  const res = await fetch(url.toString(), { credentials: 'include' });
  if (!res.ok) throw new Error(`Deltas failed: ${res.status}`);
  const body = await res.json();
  return { items: body.items as ReconDeltaItem[], page: body.page, pageSize: body.pageSize, total: body.total };
}

export function buildDeltasCsvUrl(query: ReconDeltaQuery): string {
  // Build a relative URL - works in browser, and on server we shouldn't be generating download links
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null) continue;
    params.set(k, String(v));
  }
  const queryString = params.toString();
  return `/api/v1/recon/deltas.csv${queryString ? `?${queryString}` : ''}`;
}
