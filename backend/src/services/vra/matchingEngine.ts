import {
  vraMatchAutoTotal,
  vraMatchCandidatesTotal,
  vraMatchDurationSeconds,
  vraMatchReviewTotal,
  vraMatchUnmatchedTotal,
  vraMatchExactTotal,
} from '../../utils/prometheus';

export type StatementRowLite = {
  // Canonical statement keys (from recon_statements_norm)
  statementId: string; // deterministic id (e.g., hash of row) for idempotency at persistence layer
  eventDate: string; // YYYY-MM-DD
  appId: string;
  adUnitId: string;
  country: string; // ISO-3166-1 alpha-2
  format: string; // interstitial|rewarded|banner|video|native|VAST
  paidUsd?: number; // Optional: USD-normalized paid value for proximity scoring
  // Optional exact-match identifiers where available (network dependent)
  requestId?: string;
  impressionId?: string;
  clickId?: string;
  installId?: string;
};

export type ExpectedRowLite = {
  // From recon_expected (plus optional meta hints for fuzzy matching)
  requestId: string;
  ts: string; // ISO timestamp
  expectedUsd: number; // expected_value (USD)
  // Optional hints to improve matching quality (if available via side-channel)
  appIdHint?: string;
  adUnitIdHint?: string;
  countryHint?: string;
  formatHint?: string;
};

export type MatchResult = {
  statementId: string;
  requestId: string;
  confidence: number; // 0..1
  keysUsed: 'exact' | 'fuzzy';
  reasons?: { time?: number; amount?: number; unit?: number };
};

export type MatchingOptions = {
  // Confidence threshold knobs
  autoAcceptThreshold?: number; // default 0.8
  reviewMinThreshold?: number; // default 0.5
  // Scoring weights (should sum ≈ 1 but not strictly required)
  wTime?: number; // default 0.5
  wAmount?: number; // default 0.3
  wUnit?: number; // default 0.2 (unit/app/country/format agreement)
  // Time window in seconds for fuzzy candidate selection (best‑effort; callers decide fetching window)
  timeWindowSec?: number; // default 5 * 60 (±5 minutes)
};

const DEFAULTS: Required<MatchingOptions> = {
  autoAcceptThreshold: 0.8,
  reviewMinThreshold: 0.5,
  wTime: 0.5,
  wAmount: 0.3,
  wUnit: 0.2,
  timeWindowSec: 5 * 60,
};

// Helper: normalized absolute difference in seconds clamped to window
// Uses flat bonus for same-day events to avoid unfairly penalizing early/late activity
function timeProximityScore(stmtDateISO: string, expTsISO: string, windowSec: number): number {
  const expTs = Date.parse(expTsISO);
  if (!Number.isFinite(expTs)) return 0.0;
  
  // Extract calendar date from expected timestamp (UTC)
  const expDate = new Date(expTs);
  const expDateISO = `${expDate.getUTCFullYear()}-${String(expDate.getUTCMonth() + 1).padStart(2, '0')}-${String(expDate.getUTCDate()).padStart(2, '0')}`;
  
  // Same-day match: flat bonus (no penalty for time-of-day)
  if (expDateISO === stmtDateISO) {
    return 1.0; // Perfect score for same calendar day
  }
  
  // Cross-day: calculate distance using statement's midpoint
  const stmtMid = Date.parse(stmtDateISO + 'T12:00:00Z');
  if (!Number.isFinite(stmtMid)) return 0.0;
  const dt = Math.abs(expTs - stmtMid) / 1000; // seconds
  const clamped = Math.min(dt, windowSec);
  return Math.max(0, 1.0 - clamped / windowSec); // decay for cross-day events
}

function amountProximityScore(paidUsd: number | undefined, expectedUsd: number): number {
  const paid = typeof paidUsd === 'number' && isFinite(paidUsd) ? paidUsd : undefined;
  if (!paid || expectedUsd <= 0) return 0.0;
  const relErr = Math.abs(paid - expectedUsd) / Math.max(expectedUsd, 1e-6);
  // Map relative error to [0,1] with a soft knee; 0% error → 1.0, 100% error → ~0
  const score = 1.0 / (1.0 + relErr * 4.0);
  return Math.max(0, Math.min(1, score));
}

function unitAgreementScore(stmt: StatementRowLite, exp: ExpectedRowLite): number {
  let agree = 0;
  let total = 0;
  if (exp.appIdHint) { total++; if (exp.appIdHint === stmt.appId) agree++; }
  if (exp.adUnitIdHint) { total++; if (exp.adUnitIdHint === stmt.adUnitId) agree++; }
  if (exp.countryHint) { total++; if (exp.countryHint === stmt.country) agree++; }
  if (exp.formatHint) { total++; if (exp.formatHint === stmt.format) agree++; }
  if (total === 0) return 0.0;
  return agree / total; // fraction of hint keys that agree
}

export function scoreCandidate(stmt: StatementRowLite, exp: ExpectedRowLite, opt?: MatchingOptions): number {
  const cfg = { ...DEFAULTS, ...(opt || {}) };
  const sTime = timeProximityScore(stmt.eventDate, exp.ts, cfg.timeWindowSec);
  const sAmt = amountProximityScore(stmt.paidUsd, exp.expectedUsd);
  const sUnit = unitAgreementScore(stmt, exp);
  const score = cfg.wTime * sTime + cfg.wAmount * sAmt + cfg.wUnit * sUnit;
  return Math.max(0, Math.min(1, score));
}

/**
 * Extract the date portion from an ISO timestamp (YYYY-MM-DD)
 */
function extractDateFromTs(ts: string): string {
  // Handle both "YYYY-MM-DD" and "YYYY-MM-DDTHH:MM:SS..." formats
  return ts.substring(0, 10);
}

/**
 * Build a bucket key for grouping similar rows together.
 * Uses date + appId for primary bucketing to reduce search space.
 */
function buildBucketKey(date: string, appId?: string): string {
  return `${date}|${appId || '_'}`;
}

/**
 * PERFORMANCE OPTIMIZATION: Bucket-based matching algorithm
 * 
 * Instead of O(N*M) nested loop, we:
 * 1. Build a Map of expected rows bucketed by date+appId - O(M)
 * 2. For each statement, only search its bucket + adjacent day buckets - O(N * k) where k << M
 * 
 * This reduces typical complexity from O(N*M) to approximately O(N * log(M)) for sorted data
 * or O(N * avgBucketSize) which is typically O(N) for well-distributed data.
 */
export function matchStatementsToExpected(
  statements: StatementRowLite[],
  expected: ExpectedRowLite[],
  options?: MatchingOptions
): { auto: MatchResult[]; review: MatchResult[]; unmatched: StatementRowLite[] } {
  const end = vraMatchDurationSeconds.startTimer();
  const cfg = { ...DEFAULTS, ...(options || {}) };

  const auto: MatchResult[] = [];
  const review: MatchResult[] = [];
  const unmatched: StatementRowLite[] = [];

  // STEP 1: Build request_id index for O(1) exact-match lookups
  const exactMatchIndex = new Map<string, ExpectedRowLite>();
  for (const exp of expected) {
    if (exp.requestId) {
      exactMatchIndex.set(exp.requestId, exp);
    }
  }

  // STEP 2: Build buckets by date + appId for fuzzy matching
  // Each bucket contains expected rows that could match statements with that date/appId
  const buckets = new Map<string, ExpectedRowLite[]>();
  
  for (const exp of expected) {
    const expDate = extractDateFromTs(exp.ts);
    const appId = exp.appIdHint || '_';
    
    // Add to primary bucket
    const primaryKey = buildBucketKey(expDate, appId);
    if (!buckets.has(primaryKey)) {
      buckets.set(primaryKey, []);
    }
    buckets.get(primaryKey)!.push(exp);
    
    // Also add to wildcard app bucket for cases where appId hint is missing
    const wildcardKey = buildBucketKey(expDate, '_');
    if (wildcardKey !== primaryKey) {
      if (!buckets.has(wildcardKey)) {
        buckets.set(wildcardKey, []);
      }
      buckets.get(wildcardKey)!.push(exp);
    }
  }

  // Helper: get adjacent dates for cross-midnight matching
  function getAdjacentDates(dateStr: string): string[] {
    const d = new Date(dateStr + 'T00:00:00Z');
    const prev = new Date(d.getTime() - 86400000);
    const next = new Date(d.getTime() + 86400000);
    const format = (dt: Date) => dt.toISOString().substring(0, 10);
    return [format(prev), dateStr, format(next)];
  }

  // Helper: derive a deterministic tie-break key for an expected row relative to a statement
  const tieKey = (stmt: StatementRowLite, exp: ExpectedRowLite) => {
    const paid = typeof stmt.paidUsd === 'number' && isFinite(stmt.paidUsd) ? stmt.paidUsd : Number.NaN;
    const amtDelta = Number.isFinite(paid) ? Math.abs(paid - exp.expectedUsd) : Number.POSITIVE_INFINITY;
    // Unit agreement count (higher is better) — invert to sort ascending
    let agree = 0;
    if (exp.appIdHint && exp.appIdHint === stmt.appId) agree++;
    if (exp.adUnitIdHint && exp.adUnitIdHint === stmt.adUnitId) agree++;
    if (exp.countryHint && exp.countryHint === stmt.country) agree++;
    if (exp.formatHint && exp.formatHint === stmt.format) agree++;
    const unitPenalty = 4 - agree; // fewer agreements → larger penalty
    // Primary key order per plan: requestId → timestamp → amount delta → unit hints
    return [exp.requestId || '', exp.ts || '', amtDelta, unitPenalty] as const;
  };

  const cmpTieKey = (a: ReturnType<typeof tieKey>, b: ReturnType<typeof tieKey>) => {
    // Lexicographic compare where strings ascending, numbers ascending
    if (a[0] < b[0]) return -1; if (a[0] > b[0]) return 1;
    if (a[1] < b[1]) return -1; if (a[1] > b[1]) return 1;
    if (a[2] < b[2]) return -1; if (a[2] > b[2]) return 1;
    if (a[3] < b[3]) return -1; if (a[3] > b[3]) return 1;
    return 0;
  };

  // STEP 3: Process each statement using bucket-based candidate lookup
  for (const stmt of statements) {
    // 0) Exact-key path (requestId short-circuit) - O(1) lookup
    if (stmt.requestId) {
      const expExact = exactMatchIndex.get(stmt.requestId);
      if (expExact) {
        auto.push({ statementId: stmt.statementId, requestId: expExact.requestId, confidence: 1.0, keysUsed: 'exact' });
        try { vraMatchExactTotal.inc(); } catch {}
        continue;
      }
    }

    // 1) Collect candidates from relevant buckets only (not all expected rows)
    const candidateBucketKeys = new Set<string>();
    const dates = getAdjacentDates(stmt.eventDate);
    
    for (const date of dates) {
      // Add buckets for this date with matching appId and wildcard
      candidateBucketKeys.add(buildBucketKey(date, stmt.appId));
      candidateBucketKeys.add(buildBucketKey(date, '_'));
    }

    // Collect unique candidates from all relevant buckets
    const seenRequestIds = new Set<string>();
    const candidates: ExpectedRowLite[] = [];
    
    for (const key of candidateBucketKeys) {
      const bucket = buckets.get(key);
      if (bucket) {
        for (const exp of bucket) {
          if (!seenRequestIds.has(exp.requestId)) {
            seenRequestIds.add(exp.requestId);
            candidates.push(exp);
          }
        }
      }
    }

    let best: { exp: ExpectedRowLite; score: number; reasons: { time: number; amount: number; unit: number }; key: ReturnType<typeof tieKey> } | null = null;

    // 2) Score only the candidates from relevant buckets (NOT all expected rows)
    for (const exp of candidates) {
      // Quick filter: ensure same day window heuristic (expected ts same calendar day as statement date)
      if (!exp.ts.startsWith(stmt.eventDate)) {
        // still allow if within window seconds (for late events crossing midnight)
        const prox = timeProximityScore(stmt.eventDate, exp.ts, cfg.timeWindowSec);
        if (prox <= 0) continue;
      }
      const sTime = timeProximityScore(stmt.eventDate, exp.ts, cfg.timeWindowSec);
      const sAmt = amountProximityScore(stmt.paidUsd, exp.expectedUsd);
      const sUnit = unitAgreementScore(stmt, exp);
      const score = cfg.wTime * sTime + cfg.wAmount * sAmt + cfg.wUnit * sUnit;
      vraMatchCandidatesTotal.inc();
      if (!best) {
        best = { exp, score, reasons: { time: sTime, amount: sAmt, unit: sUnit }, key: tieKey(stmt, exp) };
      } else if (score > best.score) {
        best = { exp, score, reasons: { time: sTime, amount: sAmt, unit: sUnit }, key: tieKey(stmt, exp) };
      } else if (Math.abs(score - best.score) <= 1e-9) {
        // Deterministic tie-breaker
        const k = tieKey(stmt, exp);
        if (cmpTieKey(k, best.key) < 0) {
          best = { exp, score, reasons: { time: sTime, amount: sAmt, unit: sUnit }, key: k };
        }
      }
    }

    if (!best) {
      unmatched.push(stmt);
      continue;
    }

    const keysUsed: 'exact' | 'fuzzy' = 'fuzzy';
    if (best.score >= cfg.autoAcceptThreshold) {
      auto.push({ statementId: stmt.statementId, requestId: best.exp.requestId, confidence: best.score, keysUsed, reasons: best.reasons });
      vraMatchAutoTotal.inc();
    } else if (best.score >= cfg.reviewMinThreshold) {
      review.push({ statementId: stmt.statementId, requestId: best.exp.requestId, confidence: best.score, keysUsed, reasons: best.reasons });
      vraMatchReviewTotal.inc();
    } else {
      unmatched.push(stmt);
    }
  }

  if (unmatched.length > 0) {
    // increment as a lump sum (prom-client allows inc(N))
    try { vraMatchUnmatchedTotal.inc(unmatched.length); } catch {}
  }

  end();
  return { auto, review, unmatched };
}

export default {
  scoreCandidate,
  matchStatementsToExpected,
};
