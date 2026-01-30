import Decimal from 'decimal.js';
import logger from '../../utils/logger';
import { query, insertMany } from '../../utils/postgres';
import { vraReconcileDurationSeconds, vraReconcileRowsTotal } from '../../utils/prometheus';

// Configure Decimal.js for financial precision
Decimal.set({ precision: 20, rounding: Decimal.ROUND_HALF_UP });

export interface ReconcileParams {
  from: string; // ISO
  to: string;   // ISO
  publisherId?: string; // Optional: filter by publisher (required in production for multi-tenant isolation)
  dryRun?: boolean;
}

export interface ReconcileResult {
  inserted: number;
  deltas: number; // number of delta rows computed
  amounts: {
    expectedUsd: number;
    paidUsd: number;
    unmatchedUsd: number;
    underpayUsd: number;
    timingLagUsd: number;
  };
}

const envFloat = (v: string | undefined, d: number) => {
  const n = Number(v);
  return Number.isFinite(n) ? n : d;
};

// Read tunables per invocation so tests and operators can adjust without restarts
// Note: VRA_INTRADAY_BUFFER_HOURS (default 24) is used by vraService.ts for default overview windows.
// The reconcile function requires explicit from/to params - callers should avoid including
// today's date to prevent false "Underpay" deltas from partially ingested network reports.
function readReconTunables() {
  return {
    UNDERPAY_TOL: envFloat(process.env.VRA_UNDERPAY_TOL, 0.02), // 2%
    IVT_P95_BAND_PP: envFloat(process.env.VRA_IVT_P95_BAND_PP, 2), // +2 pp over 30d p95
    FX_BAND_PCT: envFloat(process.env.VRA_FX_BAND_PCT, 0.5), // ±0.5% band
    VIEWABILITY_GAP_PP: envFloat(process.env.VRA_VIEWABILITY_GAP_PP, 15), // > 15 pp gap
  } as const;
}

export async function reconcileWindow(params: ReconcileParams): Promise<ReconcileResult> {
  const { from, to, publisherId, dryRun } = params;
  const end = vraReconcileDurationSeconds.startTimer();

  // Log warning if no publisherId provided in production (potential cross-tenant data)
  if (!publisherId && process.env.NODE_ENV === 'production') {
    logger.warn('VRA Reconcile: running without publisherId filter in production - data may span multiple tenants');
  }

  // Build parameterized query helpers for publisher filtering (SQL injection safe)
  const buildParams = (baseParams: unknown[]) => publisherId ? [...baseParams, publisherId] : baseParams;
  const publisherParamIdx = (baseCount: number) => publisherId ? `$${baseCount + 1}` : null;
  const publisherFilter = (baseParamCount: number) => publisherId ? ` AND publisher_id = ${publisherParamIdx(baseParamCount)}` : '';
  const publisherFilterAnalytics = (baseParamCount: number) => publisherId ? ` AND r.publisher_id = ${publisherParamIdx(baseParamCount)}` : '';

  try {
    const { UNDERPAY_TOL, IVT_P95_BAND_PP, FX_BAND_PCT, VIEWABILITY_GAP_PP } = readReconTunables();
    // 1) Aggregate expected USD in window (use text for precision, then Decimal)
    const expectedRows = await query<{ expected_usd: string }>(
      `SELECT COALESCE(SUM(expected_value), 0)::text AS expected_usd
         FROM recon_expected
        WHERE ts >= $1::timestamptz
          AND ts <  $2::timestamptz${publisherFilter(2)}`,
      buildParams([from, to])
    );
    const expectedUsdDec = new Decimal(expectedRows.rows[0]?.expected_usd || '0');
    const expectedUsd = expectedUsdDec.toNumber();

    if (expectedUsdDec.lte(0)) {
      end();
      return { inserted: 0, deltas: 0, amounts: { expectedUsd: 0, paidUsd: 0, unmatchedUsd: 0, underpayUsd: 0, timingLagUsd: 0 } };
    }

    // 2) Aggregate paid USD from analytics_revenue_events for only those request_ids referenced by recon_expected within the window
    // Note: Using same param index for both outer and inner query publisher filters since they use the same value
    const paidRows = await query<{ paid_usd: string }>(
      `SELECT COALESCE(SUM(revenue_usd), 0)::text AS paid_usd
         FROM analytics_revenue_events r
        WHERE r.observed_at >= $1::timestamptz
          AND r.observed_at <  $2::timestamptz${publisherFilterAnalytics(2)}
          AND r.metadata->>'request_id' IN (
                SELECT request_id
                  FROM recon_expected
                 WHERE ts >= $1::timestamptz
                   AND ts <  $2::timestamptz${publisherFilter(2)}
          )`,
      buildParams([from, to])
    );
    const paidUsdDec = new Decimal(paidRows.rows[0]?.paid_usd || '0');
    const paidUsd = paidUsdDec.toNumber();

    // 3) Unmatched expected (timing lag candidate): expected rows with no paid event in the window
    const unmatchedRows = await query<{ unmatched_usd: string }>(
      `SELECT COALESCE(SUM(e.expected_value), 0)::text AS unmatched_usd
         FROM recon_expected e
        WHERE e.ts >= $1::timestamptz
          AND e.ts <  $2::timestamptz${publisherFilter(2)}
          AND NOT EXISTS (
                SELECT 1
                  FROM analytics_revenue_events r
                 WHERE r.metadata->>'request_id' = e.request_id
                   AND r.observed_at >= $1::timestamptz
                   AND r.observed_at <  $2::timestamptz${publisherFilterAnalytics(2)}
          )`,
      buildParams([from, to])
    );
    const unmatchedUsdDec = new Decimal(unmatchedRows.rows[0]?.unmatched_usd || '0');
    const unmatchedUsd = unmatchedUsdDec.toNumber();

    // 4) Classify deltas using Decimal for precision (coarse aggregate pass)
    const gapDec = Decimal.max(0, expectedUsdDec.minus(paidUsdDec));
    const timingLagUsdDec = Decimal.max(0, Decimal.min(unmatchedUsdDec, gapDec));
    const residualGapDec = Decimal.max(0, gapDec.minus(timingLagUsdDec));
    const tolAmountDec = expectedUsdDec.times(UNDERPAY_TOL);
    const underpayUsdDec = residualGapDec.gt(tolAmountDec) ? residualGapDec : new Decimal(0);
    
    const timingLagUsd = timingLagUsdDec.toNumber();
    const underpayUsd = underpayUsdDec.toNumber();

    const rows: Array<{
      kind: string;
      amount: number;
      currency: string;
      reason_code: string;
      window_start: string;
      window_end: string;
      evidence_id: string;
      confidence: number;
    }> = [];

    const evidenceBase = `win:${from}..${to}`;
    if (timingLagUsdDec.gt(0)) {
      rows.push({
        kind: 'timing_lag',
        amount: Number(timingLagUsdDec.toFixed(6)),
        currency: 'USD',
        reason_code: 'timing_lag_unmatched',
        window_start: from,
        window_end: to,
        evidence_id: `${evidenceBase}:lag`,
        confidence: 0.6,
      });
    }
    if (underpayUsdDec.gt(0)) {
      rows.push({
        kind: 'underpay',
        amount: Number(underpayUsdDec.toFixed(6)),
        currency: 'USD',
        reason_code: 'agg_underpay',
        window_start: from,
        window_end: to,
        evidence_id: `${evidenceBase}:underpay`,
        confidence: 0.7,
      });
    }

    // 5) IVT outlier rule (approximation): compare current window IVT% vs 30-day p95 + band
    try {
      const band = Math.max(0, IVT_P95_BAND_PP) / 100; // convert pp -> fraction
      // Baseline p95 over the 30 days before window start
      const baselineRows = await query<{ p95: string; cnt: string }>(
        `WITH daily AS (
             SELECT event_date,
                    CASE WHEN SUM(paid) = 0 THEN 0::float8
                         ELSE SUM(COALESCE(ivt_adjustments, 0)) / SUM(paid)
                    END AS rate
               FROM recon_statements_norm
              WHERE event_date >= (date($1::timestamptz) - INTERVAL '30 days')
                AND event_date <  date($1::timestamptz)${publisherFilter(1)}
              GROUP BY event_date)
         SELECT COALESCE(percentile_disc(0.95) WITHIN GROUP (ORDER BY rate), 0)::float8 AS p95,
                COUNT(*)::bigint AS cnt
           FROM daily`,
        buildParams([from])
      );
      const baselineCount = Number(baselineRows.rows[0]?.cnt ?? 0);
      if (baselineCount > 0) {
        const p95 = Number(baselineRows.rows[0]?.p95 ?? 0);
        const currentRows = await query<{ rate: string }>(
          `SELECT CASE WHEN SUM(paid) = 0 THEN 0::float8
                       ELSE SUM(COALESCE(ivt_adjustments, 0)) / SUM(paid)
                  END AS rate
             FROM recon_statements_norm
            WHERE event_date >= date($1::timestamptz)
              AND event_date <  date($2::timestamptz)${publisherFilter(2)}`,
          buildParams([from, to])
        );
        const currentRate = Number(currentRows.rows[0]?.rate ?? 0);
        if (currentRate > p95 + band) {
          rows.push({
            kind: 'ivt_outlier',
            amount: 0.0, // classification without monetary assignment
            currency: 'USD',
            reason_code: 'ivt_p95_band_exceeded',
            window_start: from,
            window_end: to,
            evidence_id: `${evidenceBase}:ivt`,
            confidence: 0.6,
          });
        }
      }
    } catch (e) {
      // if CH query fails, skip ivt rule silently (degrade to no insight)
    }

    // 6) FX mismatch rule (approximation): compare current avg exchange_rate vs 30d median per currency
    try {
      const band = Math.max(0, FX_BAND_PCT) / 100; // convert percent -> fraction
      // Baseline median exchange rates per currency over 30 days before window start
      const fxBaseline = await query<{ cur: string; med_rate: string }>(
        `SELECT revenue_currency AS cur,
                COALESCE(percentile_disc(0.5) WITHIN GROUP (ORDER BY exchange_rate), 0)::float8 AS med_rate
           FROM analytics_revenue_events
          WHERE observed_at >= ($1::timestamptz - INTERVAL '30 days')
            AND observed_at <  $1::timestamptz
            AND revenue_currency <> 'USD'${publisherFilterAnalytics(1)}
          GROUP BY cur`,
        buildParams([from])
      );
      const baseMap = new Map<string, number>();
      for (const r of fxBaseline.rows) baseMap.set(r.cur, Number(r.med_rate));

      if (baseMap.size > 0) {
        const fxCurrent = await query<{ cur: string; avg_rate: string }>(
          `SELECT revenue_currency AS cur,
                  AVG(exchange_rate)::float8 AS avg_rate
             FROM analytics_revenue_events r
            WHERE r.observed_at >= $1::timestamptz
              AND r.observed_at <  $2::timestamptz
              AND r.revenue_currency <> 'USD'${publisherFilterAnalytics(2)}
            GROUP BY cur`,
          buildParams([from, to])
        );
        for (const r of fxCurrent.rows) {
          const cur = r.cur;
          const base = baseMap.get(cur);
          const avg = Number(r.avg_rate);
          if (!base || base <= 0) continue;
          const dev = Math.abs(avg - base) / base;
          // Use epsilon to avoid floating drift causing equality to trigger
          const EPS = 1e-9;
          if (dev - band > EPS) {
            rows.push({
              kind: 'fx_mismatch',
              amount: 0.0,
              currency: 'USD',
              reason_code: `fx_band_exceeded_${cur}`,
              window_start: from,
              window_end: to,
              evidence_id: `${evidenceBase}:fx:${cur}`,
              confidence: 0.6,
            });
          }
        }
      }
    } catch (e) {
      // Skip FX rule on errors (degrade to no insight)
    }

    // 7) Viewability gap rule (requires viewability payloads populated in recon_expected)
    try {
      const gap = Math.max(0, VIEWABILITY_GAP_PP) / 100; // convert pp -> fraction
      const rowsView = await query<{ om: string; stmt: string }>(
        `SELECT 
             AVG(
               CASE WHEN viewability ? 'om_viewable_pct'
                    THEN (viewability->>'om_viewable_pct')::float8
                    ELSE NULL
               END
             ) AS om,
             AVG(
               CASE WHEN viewability ? 'statement_viewable_pct'
                    THEN (viewability->>'statement_viewable_pct')::float8
                    ELSE NULL
               END
             ) AS stmt
           FROM recon_expected
          WHERE ts >= $1::timestamptz
            AND ts <  $2::timestamptz${publisherFilter(2)}`,
        buildParams([from, to])
      );
      const om = Number(rowsView.rows[0]?.om ?? NaN);
      const stmt = Number(rowsView.rows[0]?.stmt ?? NaN);
      if (Number.isFinite(om) && Number.isFinite(stmt)) {
        // Use epsilon to avoid emitting when exactly on threshold due to float drift
        const EPS = 1e-9;
        if (Math.abs(om - stmt) - gap > EPS) {
          rows.push({
            kind: 'viewability_gap',
            amount: 0.0,
            currency: 'USD',
            reason_code: 'viewability_gap_pp',
            window_start: from,
            window_end: to,
            evidence_id: `${evidenceBase}:viewability`,
            confidence: 0.6,
          });
        }
      }
    } catch (e) {
      // Skip viewability rule on errors or missing JSON fields
    }

    // Metrics
    for (const r of rows) {
      try { vraReconcileRowsTotal.inc({ kind: r.kind }, 1); } catch {}
    }

    if (!dryRun && rows.length > 0) {
      try {
        // Include publisher_id if provided for multi-tenant isolation
        const columns = publisherId 
          ? ['publisher_id', 'kind', 'amount', 'currency', 'reason_code', 'window_start', 'window_end', 'evidence_id', 'confidence']
          : ['kind', 'amount', 'currency', 'reason_code', 'window_start', 'window_end', 'evidence_id', 'confidence'];
        
        const values = rows.map((r) => publisherId
          ? [
              publisherId,
              r.kind,
              r.amount,
              r.currency,
              r.reason_code,
              r.window_start,
              r.window_end,
              r.evidence_id,
              r.confidence,
            ]
          : [
              r.kind,
              r.amount,
              r.currency,
              r.reason_code,
              r.window_start,
              r.window_end,
              r.evidence_id,
              r.confidence,
            ]
        );

        await insertMany(
          'recon_deltas',
          columns,
          values,
          { onConflictColumns: ['evidence_id'], ignoreConflicts: true }
        );
      } catch (e) {
        logger.warn('VRA Reconcile: failed to insert recon_deltas', { error: (e as Error)?.message, rows: rows.length });
        end();
        return { inserted: 0, deltas: rows.length, amounts: { expectedUsd, paidUsd, unmatchedUsd, underpayUsd, timingLagUsd } };
      }
    }

    end();
    return { inserted: dryRun ? 0 : rows.length, deltas: rows.length, amounts: { expectedUsd, paidUsd, unmatchedUsd, underpayUsd, timingLagUsd } };
  } catch (e) {
    end();
    logger.warn('VRA Reconcile: error during reconciliation (degrade to no-op)', { error: (e as Error)?.message });
    return { inserted: 0, deltas: 0, amounts: { expectedUsd: 0, paidUsd: 0, unmatchedUsd: 0, underpayUsd: 0, timingLagUsd: 0 } };
  }
}

export default { reconcileWindow };
