import crypto from 'node:crypto';
import logger from '../../utils/logger';
import { query, insertMany } from '../../utils/postgres';
import { ExpectedRowLite, StatementRowLite, matchStatementsToExpected, MatchingOptions } from './matchingEngine';
import { vraMatchReviewPersistedTotal } from '../../utils/prometheus';

type CHStatementRow = {
  event_date: string;
  app_id: string;
  ad_unit_id: string;
  country: string;
  format: string;
  paid?: string; // Decimal string
  currency?: string;
  report_id?: string;
  network?: string;
};

type CHExpectedRow = {
  request_id: string;
  ts: string;
  expected_value: string; // Decimal string
};

export interface MatchBatchParams {
  from: string; // ISO
  to: string;   // ISO
  publisherId: string; // REQUIRED: tenant isolation - prevents cross-publisher data mixing
  limitStatements?: number; // default 100k
  limitExpected?: number;   // default 100k
  dryRun?: boolean;
  options?: MatchingOptions;
  persistReview?: boolean; // when true, persist review-band matches into recon_match_review (default: true)
}

function makeStatementId(r: CHStatementRow): string {
  const base = [
    r.event_date,
    r.app_id,
    r.ad_unit_id,
    r.country,
    r.format,
    r.currency || '',
    r.report_id || '',
    r.network || '',
  ].join('|');
  return crypto.createHash('sha256').update(base).digest('hex');
}

export async function runMatchingBatch(params: MatchBatchParams): Promise<{ auto: number; review: number; unmatched: number; inserted: number; reviewPersisted: number }> {
  const from = params.from;
  const to = params.to;
  const publisherId = params.publisherId;
  
  // SECURITY: publisherId is REQUIRED for tenant isolation
  if (!publisherId || typeof publisherId !== 'string' || publisherId.trim() === '') {
    throw new Error('publisherId is required for tenant isolation - refusing to process cross-tenant batch');
  }
  
  const limitStatements = Math.min(Math.max(params.limitStatements ?? 100_000, 1), 1_000_000);
  const limitExpected = Math.min(Math.max(params.limitExpected ?? 100_000, 1), 1_000_000);
  const dryRun = params.dryRun === true;
  const matchOpts: MatchingOptions = {
    // Favor time in absence of amount/unit hints in this scaffold; keep auto threshold high
    wTime: 1.0,
    wAmount: 0.0,
    wUnit: 0.0,
    autoAcceptThreshold: 0.95,
    reviewMinThreshold: 0.3,
    ...(params.options || {}),
  };

  // Fetch statements (day-granular) and expected (timestamped) in the window
  // SECURITY: Filter by publisher_id to prevent cross-tenant data contamination
  const stmtRowsResult = await query<CHStatementRow>
    (
      `
      SELECT 
        to_char(event_date, 'YYYY-MM-DD') AS event_date,
        app_id,
        ad_unit_id,
        country,
        format,
        paid::text AS paid,
        currency,
        report_id,
        network
      FROM recon_statements_norm
      WHERE event_date >= $1::date
        AND event_date <  $2::date
        AND publisher_id = $3
      ORDER BY event_date ASC
      LIMIT $4
    `,
      [from, to, publisherId, limitStatements]
    );
  const stmtRows = stmtRowsResult.rows;

  const expRowsResult = await query<CHExpectedRow>(
    `
      SELECT request_id,
             to_char(ts, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS ts,
             expected_value::text AS expected_value
        FROM recon_expected
       WHERE ts >= $1::timestamptz
         AND ts <  $2::timestamptz
         AND publisher_id = $3
       ORDER BY ts ASC
       LIMIT $4
    `,
    [from, to, publisherId, limitExpected]
  );
  const expRows = expRowsResult.rows;

  if (stmtRows.length === 0 || expRows.length === 0) {
    return { auto: 0, review: 0, unmatched: stmtRows.length, inserted: 0, reviewPersisted: 0 };
  }

  const stmts: StatementRowLite[] = stmtRows.map((r) => ({
    statementId: makeStatementId(r),
    eventDate: r.event_date,
    appId: r.app_id,
    adUnitId: r.ad_unit_id,
    country: r.country,
    format: r.format,
    // paidUsd: undefined in scaffold; could integrate FX normalization later
  }));

  const exps: ExpectedRowLite[] = expRows.map((r) => ({
    requestId: r.request_id,
    ts: r.ts,
    expectedUsd: Number(r.expected_value) || 0,
  }));

  const { auto, review, unmatched } = matchStatementsToExpected(stmts, exps, matchOpts);

  // Persist only auto-accepted matches for now
  let inserted = 0;
  let reviewPersisted = 0;
  if (!dryRun && auto.length > 0) {
    try {
      await insertMany(
        'recon_match',
        ['statement_id', 'request_id', 'link_confidence', 'keys_used'],
        auto.map((m) => [
          m.statementId,
          m.requestId,
          Number(m.confidence.toFixed(2)),
          m.keysUsed,
        ]),
        { onConflictColumns: ['statement_id', 'request_id'], ignoreConflicts: true }
      );
      inserted = auto.length;
    } catch (e) {
      logger.warn('VRA MatchingBatch: failed to insert recon_match', { error: (e as Error)?.message, rows: auto.length });
      inserted = 0;
    }
  }

  // Persist review-band matches for analyst review (default: enabled for audit trail)
  const persistReview = params.persistReview !== false; // default true
  if (!dryRun && persistReview && review.length > 0) {
    try {
      await insertMany(
        'recon_match_review',
        ['statement_id', 'request_id', 'link_confidence', 'keys_used', 'reasons'],
        review.map((m) => [
          m.statementId,
          m.requestId,
          Number(m.confidence.toFixed(2)),
          m.keysUsed,
          JSON.stringify(m.reasons || {}),
        ])
      );
      reviewPersisted = review.length;
      try { vraMatchReviewPersistedTotal.inc(review.length); } catch {}
    } catch (e) {
      logger.warn('VRA MatchingBatch: failed to insert recon_match_review', { error: (e as Error)?.message, rows: review.length });
      reviewPersisted = 0;
    }
  }

  return { auto: auto.length, review: review.length, unmatched: unmatched.length, inserted, reviewPersisted };
}

export default { runMatchingBatch };
