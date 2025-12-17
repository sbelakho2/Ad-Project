import logger from '../utils/logger';
import { insertMany } from '../utils/postgres';

export type PerformanceMetricRecord = {
  observedAt: Date;
  metricType: string;
  metricValue: number;
  requestId?: string | null;
  publisherId?: string | null;
  adapterId?: string | null;
  errorCode?: string | null;
  errorMessage?: string | null;
  metadata?: Record<string, unknown> | null;
};

const TABLE = 'analytics_performance_metrics';
const COLUMNS = [
  'observed_at',
  'publisher_id',
  'adapter_id',
  'metric_type',
  'metric_value',
  'request_id',
  'error_code',
  'error_message',
  'metadata',
] as const;

const MAX_BUFFER = Number.parseInt(process.env.PERF_METRICS_BUFFER_MAX || '5000', 10);
const FLUSH_DELAY_MS = Number.parseInt(process.env.PERF_METRICS_FLUSH_DELAY_MS || '1000', 10);

let buffer: PerformanceMetricRecord[] = [];
let flushTimer: NodeJS.Timeout | null = null;

const scheduleFlush = (): void => {
  if (flushTimer) return;
  flushTimer = setTimeout(() => {
    flushTimer = null;
    void flush();
  }, FLUSH_DELAY_MS);

  // Don't keep the process alive for best-effort telemetry.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (flushTimer as any).unref?.();
};

export const recordPerformanceMetric = (record: PerformanceMetricRecord): void => {
  try {
    if (!record || typeof record !== 'object') return;
    if (!record.metricType || typeof record.metricType !== 'string') return;
    if (typeof record.metricValue !== 'number' || !Number.isFinite(record.metricValue)) return;

    if (buffer.length >= MAX_BUFFER) {
      // Drop when saturated; this must never backpressure production paths.
      return;
    }

    buffer.push(record);
    scheduleFlush();
  } catch {
    // best-effort only
  }
};

export const flush = async (): Promise<void> => {
  const batch = buffer;
  if (!batch.length) return;

  buffer = [];

  const rows = batch.map((r) => [
    r.observedAt,
    r.publisherId ?? null,
    r.adapterId ?? null,
    r.metricType,
    r.metricValue,
    r.requestId ?? null,
    r.errorCode ?? null,
    r.errorMessage ?? null,
    r.metadata ?? null,
  ]);

  try {
    await insertMany(TABLE, [...COLUMNS], rows);
  } catch (error) {
    logger.warn('[PerfMetrics] Failed to flush performance metrics (dropping batch)', {
      error,
      rows: rows.length,
    });
  }
};
