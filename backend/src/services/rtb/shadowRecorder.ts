import { ExperimentArm, ExperimentMode } from '../../types/migration';
import { query } from '../../utils/postgres';
import logger from '../../utils/logger';

export type OutcomeStatus = 'win' | 'no_fill' | 'error' | 'skipped';

export interface CandidateBidSnapshot {
  adapter: string;
  status: 'bid' | 'nobid';
  cpm?: number;
  latencyMs?: number;
  reason?: string;
}

export interface ShadowOutcomeRecord {
  experimentId: string;
  requestId: string;
  placementId?: string | null;
  arm: ExperimentArm;
  mode: ExperimentMode;
  status: OutcomeStatus;
  adapterName?: string;
  adapterId?: string;
  bidCpm?: number;
  currency?: string;
  latencyMs?: number;
  mirrorPercent?: number;
  bids?: CandidateBidSnapshot[];
  metadata?: Record<string, unknown>;
  errorReason?: string;
}

// Simple batching queue with size/time flush and payload size limits
type PendingRow = [
  string, // experiment_id
  string, // request_id
  string | null, // placement_id
  string, // arm
  string, // mode
  string, // status
  string | null, // adapter_name
  string | null, // adapter_id
  number | null, // bid_cpm
  string, // currency
  number | null, // latency_ms
  number | null, // mirror_percent
  string, // bids json
  string, // metadata json
  string | null // error_reason
];

const queue: PendingRow[] = [];
let flushing = false;
let timer: NodeJS.Timeout | null = null;

const MAX_QUEUE = Math.max(100, +(process.env.RTB_SHADOW_MAX_QUEUE || '500'));
const FLUSH_MS = Math.max(250, +(process.env.RTB_SHADOW_FLUSH_MS || '1000'));
const MAX_JSON_LEN = Math.max(2_000, +(process.env.RTB_SHADOW_MAX_JSON || '20000'));
const MAX_BIDS_PER_RECORD = 25;
const MAX_METADATA_KEYS = 50;
const MAX_STRING_VALUE_LEN = 500;

/**
 * SECURITY FIX: Safe JSON truncation that produces valid JSON
 * 
 * Instead of slicing the stringified JSON (which produces invalid JSON),
 * we truncate the data BEFORE stringifying by:
 * 1. Limiting array lengths
 * 2. Truncating string values
 * 3. Removing nested objects beyond a certain depth
 * 4. Dropping keys if the object is too large
 */
function safeJsonTruncate(data: unknown, maxLen: number): string {
  if (data === null || data === undefined) {
    return 'null';
  }

  // First, try direct stringify
  const direct = JSON.stringify(data);
  if (direct.length <= maxLen) {
    return direct;
  }

  // Need to truncate - work with a deep clone
  const truncated = truncateValue(data, 3); // max depth of 3
  const result = JSON.stringify(truncated);
  
  // If still too long, return a minimal placeholder
  if (result.length > maxLen) {
    return JSON.stringify({ _truncated: true, _reason: 'payload_too_large' });
  }
  
  return result;
}

/**
 * Recursively truncate a value to fit within limits
 */
function truncateValue(value: unknown, maxDepth: number): unknown {
  if (value === null || value === undefined) {
    return null;
  }

  if (maxDepth <= 0) {
    return '[depth_exceeded]';
  }

  if (typeof value === 'string') {
    return value.length > MAX_STRING_VALUE_LEN
      ? value.substring(0, MAX_STRING_VALUE_LEN) + '...'
      : value;
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return value;
  }

  if (Array.isArray(value)) {
    // Limit array size and recursively truncate elements
    const limited = value.slice(0, MAX_BIDS_PER_RECORD);
    return limited.map(item => truncateValue(item, maxDepth - 1));
  }

  if (typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    const keys = Object.keys(obj).slice(0, MAX_METADATA_KEYS);
    const result: Record<string, unknown> = {};
    
    for (const key of keys) {
      result[key] = truncateValue(obj[key], maxDepth - 1);
    }
    
    if (Object.keys(obj).length > MAX_METADATA_KEYS) {
      result._truncated_keys = Object.keys(obj).length - MAX_METADATA_KEYS;
    }
    
    return result;
  }

  // For other types (functions, symbols, etc.), return a placeholder
  return '[unsupported_type]';
}

/**
 * Truncate bids array safely - produces valid JSON
 */
function truncateBids(bids: CandidateBidSnapshot[] | undefined): string {
  if (!bids || bids.length === 0) {
    return '[]';
  }
  
  // Take only the first N bids and truncate each
  const limited = bids.slice(0, MAX_BIDS_PER_RECORD).map(bid => ({
    adapter: bid.adapter?.substring(0, 64) || 'unknown',
    status: bid.status,
    cpm: bid.cpm,
    latencyMs: bid.latencyMs,
    reason: bid.reason?.substring(0, 100),
  }));
  
  return safeJsonTruncate(limited, MAX_JSON_LEN);
}

/**
 * Truncate metadata safely - produces valid JSON
 */
function truncateMetadata(metadata: Record<string, unknown> | undefined): string {
  if (!metadata || Object.keys(metadata).length === 0) {
    return '{}';
  }
  
  return safeJsonTruncate(metadata, MAX_JSON_LEN);
}

async function flush(): Promise<void> {
  if (flushing || queue.length === 0) return;
  flushing = true;
  try {
    const batch: PendingRow[] = queue.splice(0, Math.min(queue.length, MAX_QUEUE));
    const valuesSql = batch
      .map(
        (_r, i) =>
          `($${i * 15 + 1},$${i * 15 + 2},$${i * 15 + 3},$${i * 15 + 4},$${i * 15 + 5},$${i * 15 + 6},$${i * 15 + 7},$${i * 15 + 8},$${i * 15 + 9},$${i * 15 + 10},$${i * 15 + 11},$${i * 15 + 12},$${i * 15 + 13}::jsonb,$${i * 15 + 14}::jsonb,$${i * 15 + 15})`
      )
      .join(',');
    const flatParams = batch.flat();
    await query(
      `INSERT INTO migration_shadow_outcomes (
         experiment_id,request_id,placement_id,arm,mode,status,adapter_name,adapter_id,bid_cpm,currency,latency_ms,mirror_percent,bids,metadata,error_reason
       ) VALUES ${valuesSql}`,
      flatParams
    );
  } catch (error) {
    logger.warn('Failed to batch-insert migration shadow outcomes', { error });
  } finally {
    flushing = false;
  }
}

function schedule(): void {
  if (timer) return;
  timer = setTimeout(() => {
    timer = null;
    void flush();
  }, FLUSH_MS).unref();
}

export const recordShadowOutcome = async (outcome: ShadowOutcomeRecord): Promise<void> => {
  try {
    // SECURITY FIX: Use safe truncation that produces valid JSON
    // Previous implementation used .slice() on JSON strings which could produce
    // invalid JSON and cause the entire batch insert to fail (poison pill attack)
    const bidsPayload = truncateBids(outcome.bids);
    const metadataPayload = truncateMetadata(outcome.metadata);

    const row: PendingRow = [
      outcome.experimentId,
      outcome.requestId,
      outcome.placementId ?? null,
      outcome.arm,
      outcome.mode,
      outcome.status,
      outcome.adapterName ?? null,
      outcome.adapterId ?? null,
      typeof outcome.bidCpm === 'number' ? outcome.bidCpm : null,
      outcome.currency ?? 'USD',
      typeof outcome.latencyMs === 'number' ? Math.round(outcome.latencyMs) : null,
      typeof outcome.mirrorPercent === 'number' ? Math.round(outcome.mirrorPercent) : null,
      bidsPayload,
      metadataPayload,
      outcome.errorReason ?? null,
    ];

    queue.push(row);
    if (queue.length >= MAX_QUEUE) {
      // best-effort flush
      void flush();
    } else {
      schedule();
    }
  } catch (error) {
    logger.warn('Failed to queue migration shadow outcome', {
      error,
      experimentId: outcome.experimentId,
      requestId: outcome.requestId,
    });
  }
};
