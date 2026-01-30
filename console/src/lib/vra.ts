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

// Dispute types
export type DisputeStatus = 
  | 'draft' 
  | 'submitted' 
  | 'in_progress' 
  | 'resolved' 
  | 'rejected' 
  | 'cancelled';

export interface DisputeRecord {
  id: string;
  disputeId: string;
  network: string;
  amount: number;
  status: DisputeStatus;
  evidenceUri: string;
  publisherId?: string;
  contact?: string;
  notes?: string;
  networkRefId?: string;
  resolutionAmount?: number;
  resolutionNotes?: string;
  createdAt: string;
  updatedAt: string;
  submittedAt?: string;
  resolvedAt?: string;
}

export interface DisputeStats {
  total: number;
  byStatus: Record<DisputeStatus, number>;
  totalAmountDisputed: number;
  totalAmountResolved: number;
  avgResolutionDays: number | null;
}

export type DisputeListQuery = {
  network?: string;
  status?: DisputeStatus;
  page?: number;
  page_size?: number;
};

// Review Queue types
export interface ReviewQueueItem {
  id: string;
  statementId: string;
  requestId: string;
  confidence: number;
  keysUsed: string;
  reasons: Record<string, unknown>;
  createdAt: string;
  expectedAmount: number;
  statementAmount: number;
  networkType: string;
  eventDate: string;
}

export type ReviewQueueQuery = {
  min_conf?: number;
  max_conf?: number;
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

// ─────────────────────────────────────────────────────────────────────────────
// Dispute API functions
// ─────────────────────────────────────────────────────────────────────────────

export async function listDisputes(query: DisputeListQuery = {}): Promise<{
  items: DisputeRecord[];
  pagination: { page: number; pageSize: number; total: number; totalPages: number };
}> {
  const url = new URL('/api/v1/recon/disputes', window.location.origin);
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null) continue;
    url.searchParams.set(k, String(v));
  }
  const res = await fetch(url.toString(), { credentials: 'include' });
  if (!res.ok) throw new Error(`List disputes failed: ${res.status}`);
  const body = await res.json();
  return body.data;
}

export async function getDispute(id: string): Promise<DisputeRecord> {
  const res = await fetch(`/api/v1/recon/disputes/${id}`, { credentials: 'include' });
  if (!res.ok) throw new Error(`Get dispute failed: ${res.status}`);
  const body = await res.json();
  return body.data;
}

export async function getDisputeStats(): Promise<DisputeStats> {
  const res = await fetch('/api/v1/recon/disputes/stats', { credentials: 'include' });
  if (!res.ok) throw new Error(`Get dispute stats failed: ${res.status}`);
  const body = await res.json();
  return body.data;
}

export async function createDispute(data: {
  network: string;
  amount: number;
  evidenceUri: string;
  contact?: string;
  notes?: string;
}): Promise<{ disputeId: string }> {
  const res = await fetch('/api/v1/recon/disputes', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    credentials: 'include',
    body: JSON.stringify(data),
  });
  if (!res.ok) throw new Error(`Create dispute failed: ${res.status}`);
  const body = await res.json();
  return body.data;
}

export async function submitDispute(id: string): Promise<void> {
  const res = await fetch(`/api/v1/recon/disputes/${id}/submit`, {
    method: 'POST',
    credentials: 'include',
  });
  if (!res.ok) throw new Error(`Submit dispute failed: ${res.status}`);
}

export async function cancelDispute(id: string): Promise<void> {
  const res = await fetch(`/api/v1/recon/disputes/${id}/cancel`, {
    method: 'POST',
    credentials: 'include',
  });
  if (!res.ok) throw new Error(`Cancel dispute failed: ${res.status}`);
}

// ─────────────────────────────────────────────────────────────────────────────
// Review Queue API functions
// ─────────────────────────────────────────────────────────────────────────────

export async function getReviewQueue(query: ReviewQueueQuery = {}): Promise<{
  items: ReviewQueueItem[];
  pagination: { page: number; pageSize: number; total: number; totalPages: number };
  filters: { minConf: number; maxConf: number };
}> {
  const url = new URL('/api/v1/recon/review-queue', window.location.origin);
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null) continue;
    url.searchParams.set(k, String(v));
  }
  const res = await fetch(url.toString(), { credentials: 'include' });
  if (!res.ok) throw new Error(`Get review queue failed: ${res.status}`);
  const body = await res.json();
  return body.data;
}

export async function approveReviewItem(id: string): Promise<void> {
  const res = await fetch(`/api/v1/recon/review-queue/${id}/approve`, {
    method: 'POST',
    credentials: 'include',
  });
  if (!res.ok) throw new Error(`Approve review item failed: ${res.status}`);
}

export async function rejectReviewItem(id: string): Promise<void> {
  const res = await fetch(`/api/v1/recon/review-queue/${id}/reject`, {
    method: 'POST',
    credentials: 'include',
  });
  if (!res.ok) throw new Error(`Reject review item failed: ${res.status}`);
}
