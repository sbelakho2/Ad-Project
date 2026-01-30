import type { QueryResultRow } from 'pg';
import { query } from '../../utils/postgres';
import logger from '../../utils/logger';
import {
  ReconOverviewParams,
  ReconOverviewResult,
  ReconDeltaQuery,
  ReconDeltaItem,
  ProofsMonthlyDigest,
} from './vraTypes';
import {
  vraQueryFailTotal,
  vraEmptyResultsTotal,
  vraQueryDurationSeconds,
  vraCoveragePercent,
  vraVariancePercent,
} from '../../utils/prometheus';

// Helper to time and safely execute CH queries returning empty arrays on failure (shadow-safe)
async function safeQuery<T extends QueryResultRow = QueryResultRow>(
  op: 'overview' | 'deltas_count' | 'deltas_list' | 'monthly_digest',
  sql: string,
  params?: ReadonlyArray<unknown>
): Promise<T[]> {
  const end = vraQueryDurationSeconds.startTimer({ op });
  try {
    const result = await query<T>(sql, params);
    const rows = result?.rows || [];
    end({ success: 'true' });
    if (rows.length === 0) {
      vraEmptyResultsTotal.inc({ op });
    }
    return rows;
  } catch (err) {
    end({ success: 'false' });
    vraQueryFailTotal.inc({ op });
    logger.warn('VRA safeQuery fallback (analytics read-model unavailable)', {
      error: (err as Error)?.message,
      op,
    });
    return [] as T[];
  }
}

// Environment variable for intraday buffer (hours). Default: 24 hours to avoid false "Underpay" flags
// from partially ingested network reports for the current day.
const INTRADAY_BUFFER_HOURS = Number(process.env.VRA_INTRADAY_BUFFER_HOURS) || 24;

/**
 * Compute the default VRA overview window with intraday buffer.
 * 
 * Rationale: Network statement data (recon_statements_norm) for the current day
 * is often incomplete until T+1 or later. Including today in the reconciliation
 * window would show artificial "Underpay" deltas for any expected revenue that
 * hasn't yet been reported by the network.
 * 
 * Default window: [T-8 days midnight UTC, T-1 day midnight UTC]
 * - 7 full days of data, ending yesterday at midnight UTC
 * - Configurable via VRA_INTRADAY_BUFFER_HOURS environment variable
 */
function getDefaultOverviewWindow(): { from: string; to: string } {
  const now = new Date();
  const bufferMs = INTRADAY_BUFFER_HOURS * 60 * 60 * 1000;
  
  // End at midnight UTC of (today - buffer), i.e., yesterday at 00:00:00 UTC
  const toDate = new Date(now.getTime() - bufferMs);
  toDate.setUTCHours(0, 0, 0, 0);
  
  // Start 7 days before that
  const fromDate = new Date(toDate.getTime() - 7 * 24 * 60 * 60 * 1000);
  
  return {
    from: fromDate.toISOString(),
    to: toDate.toISOString(),
  };
}

export class VraService {
  // Overview aggregates derived from analytics_revenue_events (expected) and recon_statements_norm (paid)
  async getOverview(params: ReconOverviewParams): Promise<ReconOverviewResult> {
    const publisherId = params.publisherId;
    const appId = params.appId;
    
    // Use caller-provided dates or apply intraday buffer for defaults
    const defaultWindow = getDefaultOverviewWindow();
    const from = params.from ?? defaultWindow.from;
    const to = params.to ?? defaultWindow.to;

    // Get expected revenue from SDK events (analytics_revenue_events is the modern fact table)
    const expectedRows = await safeQuery<{
      network: string;
      format: string;
      country: string;
      impressions: string;
      expected: string;
    }>(
      'overview',
      `
      SELECT
        adapter_name AS network,
        COALESCE(ad_format::text, 'unknown') AS format,
        COALESCE(country_code, 'XX') AS country,
        COUNT(*)::bigint AS impressions,
        ROUND(COALESCE(SUM(revenue_usd), 0)::numeric, 6)::text AS expected
      FROM analytics_revenue_events
      WHERE observed_at >= $1::timestamptz
        AND observed_at <  $2::timestamptz
        AND publisher_id = $3
        ${appId ? 'AND app_id = $4' : ''}
      GROUP BY adapter_name, ad_format, country_code
      ORDER BY expected DESC
      `,
      appId ? [from, to, publisherId, appId] : [from, to, publisherId]
    );

    // Get paid revenue from network statements
    const paidRows = await safeQuery<{
      network: string;
      format: string;
      country: string;
      impressions: string;
      paid: string;
    }>(
      'overview',
      `
      SELECT
        network_type AS network,
        COALESCE(format, 'unknown') AS format,
        COALESCE(country, 'XX') AS country,
        COALESCE(SUM(impressions), 0)::bigint AS impressions,
        ROUND(COALESCE(SUM(paid), 0)::numeric, 6)::text AS paid
      FROM recon_statements_norm
      WHERE event_date >= $1::date
        AND event_date <  $2::date
        AND publisher_id = $3
        ${appId ? 'AND app_id = $4' : ''}
      GROUP BY network_type, format, country
      ORDER BY paid DESC
      `,
      appId ? [from, to, publisherId, appId] : [from, to, publisherId]
    );

    // Build lookup map for paid amounts
    const paidMap = new Map<string, { impressions: number; paid: number }>();
    for (const r of paidRows) {
      const key = `${r.network}|${r.format}|${r.country}`;
      paidMap.set(key, {
        impressions: Number(r.impressions) || 0,
        paid: Number(r.paid) || 0,
      });
    }

    // Merge expected with paid data
    const byBreakdown = expectedRows.map((r) => {
      const key = `${r.network}|${r.format}|${r.country}`;
      const paidData = paidMap.get(key);
      return {
        network: r.network,
        format: r.format,
        country: r.country,
        impressions: Number(r.impressions) || 0,
        expected: Number(r.expected) || 0,
        paid: paidData?.paid || 0,
      };
    });

    // Calculate totals
    const totals = byBreakdown.reduce(
      (acc, r) => {
        acc.impressions += r.impressions;
        acc.expected += r.expected;
        acc.paid += r.paid;
        return acc;
      },
      { impressions: 0, paid: 0, expected: 0 }
    );

    // Calculate real variance percentage
    const varianceUsd = totals.expected - totals.paid;
    const variancePercent = totals.expected > 0 
      ? Math.round((varianceUsd / totals.expected) * 10000) / 100 
      : 0;

    // Calculate real coverage: networks with statements / networks with events
    const networksWithEvents = new Set(expectedRows.map(r => r.network));
    const networksWithStatements = new Set(paidRows.map(r => r.network));
    const coveredNetworks = [...networksWithEvents].filter(n => networksWithStatements.has(n));
    const coveragePercent = networksWithEvents.size > 0
      ? Math.round((coveredNetworks.length / networksWithEvents.size) * 100)
      : (paidRows.length > 0 ? 100 : 0);

    // Best‑effort publish pilot‑gate gauges used by alerts; never throw
    try {
      vraCoveragePercent.set({ scope: 'pilot' }, coveragePercent);
      vraVariancePercent.set({ scope: 'pilot' }, Math.abs(variancePercent));
    } catch (_) {
      // ignore metric errors in lightweight/test environments
    }

    // Per-network slices for UI overview and dashboards
    const byNetworkMap = new Map<string, { network: string; impressions: number; paid: number; expected: number; variance: number }>();
    for (const r of byBreakdown) {
      const prev = byNetworkMap.get(r.network) || { network: r.network, impressions: 0, paid: 0, expected: 0, variance: 0 };
      prev.impressions += r.impressions;
      prev.paid += r.paid;
      prev.expected += r.expected;
      prev.variance = prev.expected - prev.paid;
      byNetworkMap.set(r.network, prev);
    }
    const byNetwork = Array.from(byNetworkMap.values()).sort((a, b) => b.expected - a.expected);

    return {
      coveragePercent,
      variancePercent,
      totals,
      byBreakdown,
      byNetwork,
    };
  }

  async getDeltas(query: ReconDeltaQuery): Promise<{ items: ReconDeltaItem[]; page: number; pageSize: number; total: number }> {
    const page = Math.max(1, query.page || 1);
    const pageSize = Math.min(500, Math.max(1, query.pageSize || 100));
    const from = query.from ?? new Date(Date.now() - 7 * 86400000).toISOString();
    const to = query.to ?? new Date().toISOString();

    const whereClauses: string[] = [];
    const whereParams: unknown[] = [];
    const pushClause = (template: string, value: unknown) => {
      whereParams.push(value);
      const idx = whereParams.length;
      whereClauses.push(template.replace('?', `$${idx}`));
    };
    pushClause('window_start >= ?::timestamptz', from);
    pushClause('window_end < ?::timestamptz', to);
    pushClause('publisher_id = ?', query.publisherId);
    if (query.appId) pushClause('app_id = ?', query.appId);
    if (query.kind) pushClause('kind = ?', query.kind);
    if (query.minConf != null) pushClause('confidence >= ?', query.minConf);

    const whereSql = whereClauses.length ? `WHERE ${whereClauses.join(' AND ')}` : '';

    const countRows = await safeQuery<{ total: string }>(
      'deltas_count',
      `SELECT COUNT(*)::bigint AS total FROM recon_deltas ${whereSql}`,
      whereParams
    );
    const total = Number(countRows[0]?.total || 0);

    const items = await safeQuery<{
      kind: string;
      amount: string;
      currency: string;
      reason_code: string;
      window_start: string;
      window_end: string;
      evidence_id: string;
      confidence: string;
    }>(
      'deltas_list',
      `
      SELECT kind,
             amount::text AS amount,
             currency,
             reason_code,
             TO_CHAR(window_start, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS window_start,
             TO_CHAR(window_end, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')   AS window_end,
             evidence_id,
             confidence::text AS confidence
        FROM recon_deltas
        ${whereSql}
        ORDER BY window_start DESC, evidence_id ASC
        LIMIT $${whereParams.length + 1} OFFSET $${whereParams.length + 2}
      `,
      [...whereParams, pageSize, (page - 1) * pageSize]
    );

    const mapped: ReconDeltaItem[] = items.map((r) => ({
      kind: r.kind as ReconDeltaItem['kind'],
      amount: Number(r.amount) || 0,
      currency: r.currency,
      reasonCode: r.reason_code,
      windowStart: r.window_start,
      windowEnd: r.window_end,
      evidenceId: r.evidence_id,
      confidence: Number(r.confidence) || 0,
    }));

    return { items: mapped, page, pageSize, total };
  }

  async getMonthlyDigest(month: string): Promise<ProofsMonthlyDigest | null> {
    const rows = await safeQuery<{
      month: string;
      digest: string;
      sig: string;
      coverage_pct: string;
      notes: string;
    }>(
      'monthly_digest',
      `SELECT month, digest, sig, coverage_pct::text AS coverage_pct, notes
         FROM proofs_monthly_digest
        WHERE month = $1
        LIMIT 1`,
      [month]
    );
    if (rows.length === 0) return null;
    return {
      month: rows[0].month,
      digest: rows[0].digest,
      signature: rows[0].sig,
      coveragePct: Number(rows[0].coverage_pct) || 0,
      notes: rows[0].notes,
    };
  }
}

export const vraService = new VraService();
