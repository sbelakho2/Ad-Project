export interface ReconOverviewParams {
  appId?: string;
  publisherId: string;
  from?: string; // ISO date
  to?: string;   // ISO date
}

export interface ReconOverviewItem {
  network: string;
  format: string;
  country: string;
  impressions: number;
  paid: number; // in USD (normalized) - from network statements
  expected: number; // in USD - from SDK events
}

export interface ReconOverviewResult {
  coveragePercent: number; // 0-100 - networks with statements / networks with events
  variancePercent: number; // (expected - paid) / expected * 100
  totals: {
    impressions: number;
    paid: number;
    expected: number;
  };
  byBreakdown: ReconOverviewItem[];
  // Aggregated per-network slices for UI overview and dashboards
  byNetwork?: Array<{
    network: string;
    impressions: number;
    paid: number;
    expected: number;
    variance?: number;
  }>;
}

export interface ReconDeltaItem {
  kind: 'underpay' | 'overpay' | 'missing' | 'missing_network' | 'missing_sdk' | 'viewability_gap' | 'ivt_outlier' | 'fx_mismatch' | 'timing_lag';
  amount: number; // USD
  currency: string; // ISO-4217
  reasonCode: string;
  windowStart: string; // ISO ts
  windowEnd: string;   // ISO ts
  evidenceId: string;
  confidence: number; // 0..1
  network?: string;
  resolved?: boolean;
}

export interface ReconDeltaQuery {
  publisherId: string;
  appId?: string;
  from?: string;
  to?: string;
  kind?: ReconDeltaItem['kind'];
  minConf?: number; // 0..1
  page?: number;
  pageSize?: number;
}

export interface ProofsMonthlyDigest {
  month: string; // YYYY-MM
  digest: string; // hex
  signature: string; // hex
  coveragePct: number;
  notes?: string;
}
