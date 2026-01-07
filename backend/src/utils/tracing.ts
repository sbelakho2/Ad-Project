/**
 * Lightweight Distributed Tracing Utility
 *
 * Provides span-based tracing without requiring OpenTelemetry SDK.
 * Integrates with existing request context middleware for correlation.
 *
 * Features:
 * - Span creation with parent-child relationships
 * - Automatic duration tracking
 * - Error recording
 * - Prometheus histogram integration
 * - Compatible with future OpenTelemetry migration
 */

import { randomUUID } from 'crypto';
import { Histogram, Counter } from 'prom-client';
import { asyncLocalStorage, logger } from './logger';

// ============================================
// Types
// ============================================

export interface SpanContext {
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  serviceName: string;
  operationName: string;
  startTime: number;
  attributes: Record<string, string | number | boolean>;
}

export interface SpanOptions {
  /** Operation name (e.g., 'db.query', 'http.request', 'auction.execute') */
  operationName: string;
  /** Optional parent span ID for nested operations */
  parentSpanId?: string;
  /** Initial attributes */
  attributes?: Record<string, string | number | boolean>;
}

export interface FinishedSpan extends SpanContext {
  endTime: number;
  durationMs: number;
  status: 'ok' | 'error';
  errorMessage?: string;
  errorStack?: string;
}

type SpanCallback<T> = () => T | Promise<T>;

// ============================================
// Metrics
// ============================================

const spanDurationHistogram = new Histogram({
  name: 'trace_span_duration_seconds',
  help: 'Duration of traced spans in seconds',
  labelNames: ['service', 'operation', 'status'],
  buckets: [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
});

const spanCounter = new Counter({
  name: 'trace_spans_total',
  help: 'Total number of traced spans',
  labelNames: ['service', 'operation', 'status'],
});

// ============================================
// Configuration
// ============================================

const SERVICE_NAME = process.env.SERVICE_NAME || 'apexmediation-backend';
const TRACE_SAMPLE_RATE = parseFloat(process.env.TRACE_SAMPLE_RATE || '1.0');
const TRACE_LOG_ENABLED = process.env.TRACE_LOG_ENABLED !== '0';

// ============================================
// Span Storage (for correlation)
// ============================================

const activeSpans = new Map<string, SpanContext>();

// ============================================
// Core Functions
// ============================================

/**
 * Generate a new trace ID (128-bit hex)
 */
export function generateTraceId(): string {
  return randomUUID().replace(/-/g, '');
}

/**
 * Generate a new span ID (64-bit hex)
 */
export function generateSpanId(): string {
  return randomUUID().replace(/-/g, '').slice(0, 16);
}

/**
 * Get or create trace ID from current request context
 */
export function getTraceId(): string {
  const ctx = asyncLocalStorage.getStore() as { requestId?: string; traceId?: string } | undefined;
  return ctx?.traceId || ctx?.requestId || generateTraceId();
}

/**
 * Start a new span
 */
export function startSpan(options: SpanOptions): SpanContext {
  const spanId = generateSpanId();
  const traceId = getTraceId();

  const span: SpanContext = {
    traceId,
    spanId,
    parentSpanId: options.parentSpanId,
    serviceName: SERVICE_NAME,
    operationName: options.operationName,
    startTime: Date.now(),
    attributes: options.attributes || {},
  };

  activeSpans.set(spanId, span);
  return span;
}

/**
 * End a span and record metrics
 */
export function endSpan(
  span: SpanContext,
  status: 'ok' | 'error' = 'ok',
  error?: Error
): FinishedSpan {
  const endTime = Date.now();
  const durationMs = endTime - span.startTime;
  const durationSec = durationMs / 1000;

  const finished: FinishedSpan = {
    ...span,
    endTime,
    durationMs,
    status,
    errorMessage: error?.message,
    errorStack: error?.stack,
  };

  // Remove from active spans
  activeSpans.delete(span.spanId);

  // Record metrics (only if sampling allows)
  if (Math.random() <= TRACE_SAMPLE_RATE) {
    spanDurationHistogram.observe(
      { service: span.serviceName, operation: span.operationName, status },
      durationSec
    );
    spanCounter.inc({ service: span.serviceName, operation: span.operationName, status });
  }

  // Log span completion (for debugging/audit)
  if (TRACE_LOG_ENABLED && (status === 'error' || durationMs > 1000)) {
    const logLevel = status === 'error' ? 'warn' : 'debug';
    logger[logLevel]('Span completed', {
      traceId: finished.traceId,
      spanId: finished.spanId,
      parentSpanId: finished.parentSpanId,
      operation: finished.operationName,
      durationMs: finished.durationMs,
      status: finished.status,
      ...(finished.errorMessage && { error: finished.errorMessage }),
      ...finished.attributes,
    });
  }

  return finished;
}

/**
 * Add attributes to an active span
 */
export function addSpanAttributes(
  span: SpanContext,
  attributes: Record<string, string | number | boolean>
): void {
  Object.assign(span.attributes, attributes);
}

/**
 * Wrap a synchronous or async function with span tracing
 *
 * @example
 * const result = await withSpan('db.query', async () => {
 *   return await db.query('SELECT * FROM users');
 * });
 */
export async function withSpan<T>(
  operationName: string,
  fn: SpanCallback<T>,
  options?: Partial<SpanOptions>
): Promise<T> {
  const span = startSpan({ operationName, ...options });

  try {
    const result = await fn();
    endSpan(span, 'ok');
    return result;
  } catch (error) {
    endSpan(span, 'error', error instanceof Error ? error : new Error(String(error)));
    throw error;
  }
}

/**
 * Wrap a synchronous function with span tracing
 */
export function withSpanSync<T>(
  operationName: string,
  fn: () => T,
  options?: Partial<SpanOptions>
): T {
  const span = startSpan({ operationName, ...options });

  try {
    const result = fn();
    endSpan(span, 'ok');
    return result;
  } catch (error) {
    endSpan(span, 'error', error instanceof Error ? error : new Error(String(error)));
    throw error;
  }
}

/**
 * Create a child span under a parent
 */
export function createChildSpan(parent: SpanContext, operationName: string): SpanContext {
  return startSpan({
    operationName,
    parentSpanId: parent.spanId,
    attributes: { parentOperation: parent.operationName },
  });
}

/**
 * Get currently active spans (for debugging)
 */
export function getActiveSpans(): SpanContext[] {
  return Array.from(activeSpans.values());
}

/**
 * Extract trace context for propagation to downstream services
 *
 * @example
 * const headers = getTraceHeaders();
 * await fetch(url, { headers });
 */
export function getTraceHeaders(span?: SpanContext): Record<string, string> {
  const traceId = span?.traceId || getTraceId();
  const spanId = span?.spanId || generateSpanId();

  return {
    'x-trace-id': traceId,
    'x-span-id': spanId,
    'x-parent-span-id': span?.parentSpanId || '',
    traceparent: `00-${traceId}-${spanId}-01`, // W3C Trace Context format
  };
}

/**
 * Parse incoming trace context from request headers
 */
export function parseTraceHeaders(headers: Record<string, string | string[] | undefined>): {
  traceId?: string;
  parentSpanId?: string;
} {
  const traceId = Array.isArray(headers['x-trace-id'])
    ? headers['x-trace-id'][0]
    : headers['x-trace-id'];
  const parentSpanId = Array.isArray(headers['x-span-id'])
    ? headers['x-span-id'][0]
    : headers['x-span-id'];

  // Also try W3C traceparent format
  const traceparent = Array.isArray(headers['traceparent'])
    ? headers['traceparent'][0]
    : headers['traceparent'];

  if (traceparent) {
    const parts = traceparent.split('-');
    if (parts.length >= 3) {
      return {
        traceId: parts[1],
        parentSpanId: parts[2],
      };
    }
  }

  return { traceId, parentSpanId };
}

// ============================================
// Express Middleware
// ============================================

import { Request, Response, NextFunction } from 'express';

/**
 * Express middleware to auto-create request spans
 */
export function tracingMiddleware(req: Request, res: Response, next: NextFunction): void {
  const { traceId, parentSpanId } = parseTraceHeaders(req.headers);

  const span = startSpan({
    operationName: `${req.method} ${req.route?.path || req.path}`,
    parentSpanId,
    attributes: {
      'http.method': req.method,
      'http.url': req.originalUrl,
      'http.route': req.route?.path || req.path,
      'http.user_agent': req.get('user-agent') || 'unknown',
    },
  });

  // Override trace ID if provided in headers
  if (traceId) {
    (span as SpanContext & { traceId: string }).traceId = traceId;
  }

  // Add trace ID to response headers
  res.setHeader('x-trace-id', span.traceId);
  res.setHeader('x-span-id', span.spanId);

  // End span when response finishes
  res.on('finish', () => {
    addSpanAttributes(span, {
      'http.status_code': res.statusCode,
    });
    const status = res.statusCode >= 400 ? 'error' : 'ok';
    endSpan(span, status);
  });

  next();
}

// ============================================
// Exports
// ============================================

export default {
  startSpan,
  endSpan,
  withSpan,
  withSpanSync,
  createChildSpan,
  addSpanAttributes,
  getTraceId,
  getTraceHeaders,
  parseTraceHeaders,
  getActiveSpans,
  tracingMiddleware,
};
