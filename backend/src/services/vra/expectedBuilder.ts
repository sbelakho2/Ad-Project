import { Pool } from 'pg';
import logger from '../../utils/logger';
import {
  vraExpectedBuildDurationSeconds,
  vraExpectedSeenTotal,
  vraExpectedSkippedTotal,
  vraExpectedWrittenTotal,
} from '../../utils/prometheus';

type ReceiptRow = {
  request_id: string;
  placement_id: string;
  ts: string; // ISO timestamp
  floor_cpm?: number | null;
  currency?: string | null;
  receipt_hash?: string | null;
  floors?: any | null; // JSON
};

type PaidEventRow = {
  request_id: string;
  ts: string; // ISO timestamp
  revenue_usd?: string; // Decimal as string
  revenue_currency?: string; // e.g., USD
  revenue_original?: string; // Decimal as string
};

export interface BuildExpectedParams {
  from: string; // ISO
  to: string;   // ISO
  limit?: number; // max receipts to process per call (default 10k)
}

export async function buildReconExpected(
  pgPool: Pool,
  params: BuildExpectedParams,
  options?: { dryRun?: boolean; collectMetrics?: boolean }
): Promise<{ seen: number; written: number; skipped: number }> {
  const dryRun = options?.dryRun === true;
  const collect = options?.collectMetrics !== false; // default true
  const endTimer = vraExpectedBuildDurationSeconds.startTimer();
  const from = params.from;
  const to = params.to;
  const limit = Math.min(Math.max(params.limit ?? 10000, 1), 100000);

  // 1) Fetch receipts from Postgres (append-only transparency receipts)
  // We fetch minimal columns needed; exact table name may vary by deployment (assume transparency_receipts)
  const sql = `
    SELECT 
      req_id        AS request_id,
      placement_id  AS placement_id,
      timestamp     AS ts,
      floor_cpm     AS floor_cpm,
      currency      AS currency,
      hash          AS receipt_hash
    FROM transparency_receipts
    WHERE timestamp >= $1 AND timestamp < $2
    ORDER BY timestamp ASC
    LIMIT $3`;

  let receipts: ReceiptRow[] = [];
  try {
    const r = await pgPool.query(sql.trim(), [from, to, limit]);
    receipts = (r.rows || []) as ReceiptRow[];
  } catch (err) {
    logger.warn('VRA ExpectedBuilder: failed to read receipts from Postgres (treat as none)', { error: (err as Error)?.message });
    receipts = [];
  }

  if (receipts.length === 0) {
    try {
      if (collect) {
        endTimer({ outcome: 'empty' });
      }
    } catch {}
    return { seen: 0, written: 0, skipped: 0 };
  }

  // 2) Build set of request_ids
  const reqIds = Array.from(new Set(receipts.map((r) => r.request_id).filter(Boolean)));
  if (reqIds.length === 0) {
    try {
      if (collect) {
        vraExpectedSeenTotal.inc(receipts.length);
        vraExpectedSkippedTotal.inc(receipts.length);
        endTimer({ outcome: dryRun ? 'dry_run' : 'success' });
      }
    } catch {}
    return { seen: receipts.length, written: 0, skipped: receipts.length };
  }

  // 3) Determine which ones already exist in recon_expected to keep idempotency
  let existingSet = new Set<string>();
  try {
    const existing = await pgPool.query<{ request_id: string }>(
      `SELECT request_id FROM recon_expected WHERE request_id = ANY($1::text[])`,
      [reqIds]
    );
    existingSet = new Set(existing.rows.map((x) => x.request_id));
  } catch (err) {
    logger.warn('VRA ExpectedBuilder: failed to read existing recon_expected rows', { error: (err as Error)?.message });
  }

  const candidates = receipts.filter((r) => !existingSet.has(r.request_id));
  if (candidates.length === 0) {
    try {
      if (collect) {
        vraExpectedSeenTotal.inc(receipts.length);
        vraExpectedSkippedTotal.inc(receipts.length);
        endTimer({ outcome: dryRun ? 'dry_run' : 'success' });
      }
    } catch {}
    return { seen: receipts.length, written: 0, skipped: receipts.length };
  }

  // 4) Fetch paid events from Postgres within the same window for the request_ids
  let paidEvents: PaidEventRow[] = [];
  try {
    const res = await pgPool.query<PaidEventRow>(
      `SELECT metadata->>'request_id' AS request_id,
              to_char(observed_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS ts,
              revenue_usd::text AS revenue_usd,
              revenue_original::text AS revenue_original,
              revenue_currency
         FROM analytics_revenue_events
        WHERE observed_at >= $1::timestamptz
          AND observed_at <  $2::timestamptz
          AND metadata->>'request_id' = ANY($3::text[])
          AND revenue_usd > 0`,
      [from, to, candidates.map((c) => c.request_id)]
    );
    paidEvents = res.rows;
  } catch (err) {
    logger.warn('VRA ExpectedBuilder: failed to read paid events from Postgres (treat as none)', { error: (err as Error)?.message });
  }
  const paidByReq = new Map<string, PaidEventRow>();
  for (const p of paidEvents) {
    if (!paidByReq.has(p.request_id)) paidByReq.set(p.request_id, p);
  }

  // 5) Map into recon_expected rows (insert only those with a paid event; others can be addressed in future iterations)
  const rows = candidates
    .map((rec) => {
      const paid = paidByReq.get(rec.request_id);
      if (!paid) return null; // skip for now; unmatched receipts will be handled later
      const floorsObj = rec.floors ?? {
        floor_cpm: Number.isFinite(rec.floor_cpm as number) ? rec.floor_cpm : undefined,
        currency: rec.currency || undefined,
      };
      return {
        event_date: (rec.ts || '').slice(0, 10),
        request_id: rec.request_id,
        placement_id: rec.placement_id,
        expected_value: Number(paid.revenue_usd || '0') || 0,
        currency: 'USD',
        floors: JSON.stringify(floorsObj),
        receipt_hash: rec.receipt_hash || '',
        viewability: '{}',
        ts: rec.ts,
      };
    })
    .filter(Boolean) as Array<{
      event_date: string;
      request_id: string;
      placement_id: string;
      expected_value: number;
      currency: string;
      floors: string;
      receipt_hash: string;
      viewability: string;
      ts: string;
    }>;

  if (rows.length === 0) {
    try {
      if (collect) {
        vraExpectedSeenTotal.inc(receipts.length);
        vraExpectedSkippedTotal.inc(receipts.length);
        endTimer({ outcome: dryRun ? 'dry_run' : 'success' });
      }
    } catch {}
    return { seen: receipts.length, written: 0, skipped: receipts.length };
  }

  let written = 0;
  if (!dryRun) {
    try {
      const columns = [
        'event_date',
        'request_id',
        'placement_id',
        'expected_value',
        'currency',
        'floors',
        'receipt_hash',
        'viewability',
        'ts',
      ];
      const values: string[] = [];
      const params: unknown[] = [];
      rows.forEach((row, idx) => {
        const base = idx * columns.length;
        values.push(`($${base + 1}, $${base + 2}, $${base + 3}, $${base + 4}, $${base + 5}, $${base + 6}, $${base + 7}, $${base + 8}, $${base + 9})`);
        params.push(
          row.event_date,
          row.request_id,
          row.placement_id,
          row.expected_value,
          row.currency,
          row.floors,
          row.receipt_hash,
          row.viewability,
          row.ts
        );
      });
      const sql = `INSERT INTO recon_expected (${columns.join(', ')}) VALUES ${values.join(', ')} ON CONFLICT (request_id) DO NOTHING`;
      const res = await pgPool.query(sql, params);
      written = res.rowCount ?? rows.length;
    } catch (err) {
      logger.warn('VRA ExpectedBuilder: failed to insert recon_expected rows', {
        error: (err as Error)?.message,
        rows: rows.length,
      });
      try {
        if (collect) {
          vraExpectedSeenTotal.inc(receipts.length);
          vraExpectedSkippedTotal.inc(receipts.length);
          endTimer({ outcome: 'error' });
        }
      } catch {}
      return { seen: receipts.length, written: 0, skipped: receipts.length };
    }
  } else {
    // Dry run: simulate writes for metrics/preview
    written = rows.length;
  }

  const skipped = receipts.length - written;
  try {
    if (collect) {
      vraExpectedSeenTotal.inc(receipts.length);
      vraExpectedWrittenTotal.inc(written);
      if (skipped > 0) vraExpectedSkippedTotal.inc(skipped);
      endTimer({ outcome: dryRun ? 'dry_run' : 'success' });
    }
  } catch {}

  return { seen: receipts.length, written, skipped };
}

export default { buildReconExpected };
