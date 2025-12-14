/**
 * APP TELEMETRY ADAPTER
 * 
 * Collects application-level metrics from the customer-facing backend.
 * Fetches from Prometheus metrics endpoint exposed by the backend.
 */

import { logger } from '../logger';
import { config } from '../config';

export interface APILatencyMetrics {
  endpoint: string;
  method: string;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  request_count: number;
  error_rate: number;
}

export interface PoolMetrics {
  pool_name: string;
  total_connections: number;
  idle_connections: number;
  active_connections: number;
  waiting_clients: number;
  saturation_pct: number;
}

export interface ErrorMetrics {
  error_class: string;
  count: number;
  rate_per_min: number;
  last_seen: string;
  sample_message?: string;
}

export interface QueueMetrics {
  queue_name: string;
  pending: number;
  processing: number;
  failed: number;
  throughput_per_min: number;
  oldest_message_age_sec: number;
}

export interface AppTelemetry {
  timestamp: string;
  latency: APILatencyMetrics[];
  pools: PoolMetrics[];
  errors: ErrorMetrics[];
  queues: QueueMetrics[];
  health: {
    status: 'OK' | 'DEGRADED' | 'DOWN';
    uptime_sec: number;
    version: string;
    last_deploy: string;
  };
}

/**
 * Get full app telemetry from the backend metrics endpoint
 */
export async function getAppTelemetry(): Promise<AppTelemetry> {
  const metricsUrl = config.BACKEND_METRICS_URL || 'http://localhost:8080/metrics';
  
  try {
    const response = await fetch(metricsUrl);
    if (!response.ok) {
      throw new Error(`Metrics endpoint returned ${response.status}`);
    }
    
    const metricsText = await response.text();
    
    return {
      timestamp: new Date().toISOString(),
      latency: parseLatencyMetrics(metricsText),
      pools: parsePoolMetrics(metricsText),
      errors: parseErrorMetrics(metricsText),
      queues: parseQueueMetrics(metricsText),
      health: await parseHealthMetrics(metricsText),
    };
  } catch (err) {
    logger.error('Failed to fetch app telemetry', { error: (err as Error).message });
    
    // Return empty telemetry on error
    return {
      timestamp: new Date().toISOString(),
      latency: [],
      pools: [],
      errors: [],
      queues: [],
      health: {
        status: 'DOWN',
        uptime_sec: 0,
        version: 'unknown',
        last_deploy: 'unknown',
      },
    };
  }
}

/**
 * Get API latency metrics (p50, p95, p99)
 */
export async function getAPILatency(): Promise<APILatencyMetrics[]> {
  const telemetry = await getAppTelemetry();
  return telemetry.latency;
}

/**
 * Get connection pool saturation
 */
export async function getPoolSaturation(): Promise<PoolMetrics[]> {
  const telemetry = await getAppTelemetry();
  return telemetry.pools;
}

/**
 * Get error rates by class
 */
export async function getErrorRates(): Promise<ErrorMetrics[]> {
  const telemetry = await getAppTelemetry();
  return telemetry.errors;
}

/**
 * Get queue depths and throughput
 */
export async function getQueueMetrics(): Promise<QueueMetrics[]> {
  const telemetry = await getAppTelemetry();
  return telemetry.queues;
}

/**
 * Get slow endpoints (p99 > threshold)
 */
export async function getSlowEndpoints(thresholdMs: number = 1000): Promise<APILatencyMetrics[]> {
  const latency = await getAPILatency();
  return latency.filter(m => m.p99_ms > thresholdMs).sort((a, b) => b.p99_ms - a.p99_ms);
}

/**
 * Get saturated pools (> 80% utilization)
 */
export async function getSaturatedPools(thresholdPct: number = 80): Promise<PoolMetrics[]> {
  const pools = await getPoolSaturation();
  return pools.filter(p => p.saturation_pct > thresholdPct).sort((a, b) => b.saturation_pct - a.saturation_pct);
}

/**
 * Get high error rate endpoints
 */
export async function getHighErrorEndpoints(thresholdPct: number = 5): Promise<ErrorMetrics[]> {
  const errors = await getErrorRates();
  return errors.filter(e => e.rate_per_min > thresholdPct).sort((a, b) => b.rate_per_min - a.rate_per_min);
}

// Parser functions for Prometheus text format

function parseLatencyMetrics(metricsText: string): APILatencyMetrics[] {
  const metrics: APILatencyMetrics[] = [];
  const endpointMap = new Map<string, Partial<APILatencyMetrics>>();
  
  // Parse http_request_duration_seconds_bucket metrics
  const bucketRegex = /http_request_duration_seconds_bucket\{([^}]+)\}\s+([\d.]+)/g;
  const sumRegex = /http_request_duration_seconds_sum\{([^}]+)\}\s+([\d.]+)/g;
  const countRegex = /http_request_duration_seconds_count\{([^}]+)\}\s+([\d.]+)/g;
  
  // Parse count (request_count)
  let match;
  while ((match = countRegex.exec(metricsText)) !== null) {
    const labels = parseLabels(match[1]);
    const key = `${labels.method}:${labels.path || labels.route}`;
    
    if (!endpointMap.has(key)) {
      endpointMap.set(key, {
        endpoint: labels.path || labels.route || 'unknown',
        method: labels.method || 'GET',
        request_count: parseFloat(match[2]),
        p50_ms: 0,
        p95_ms: 0,
        p99_ms: 0,
        error_rate: 0,
      });
    } else {
      const existing = endpointMap.get(key)!;
      existing.request_count = parseFloat(match[2]);
    }
  }
  
  // Parse histogram buckets to calculate percentiles
  // This is a simplified version - real implementation would use proper histogram calculation
  while ((match = bucketRegex.exec(metricsText)) !== null) {
    const labels = parseLabels(match[1]);
    const key = `${labels.method}:${labels.path || labels.route}`;
    const le = parseFloat(labels.le || '0');
    const count = parseFloat(match[2]);
    
    if (endpointMap.has(key) && le !== Infinity) {
      const existing = endpointMap.get(key)!;
      // Rough percentile estimation based on bucket
      if (le <= 0.05) existing.p50_ms = le * 1000;
      if (le <= 0.1) existing.p95_ms = le * 1000;
      if (le <= 0.5) existing.p99_ms = le * 1000;
    }
  }
  
  endpointMap.forEach(m => {
    metrics.push(m as APILatencyMetrics);
  });
  
  return metrics;
}

function parsePoolMetrics(metricsText: string): PoolMetrics[] {
  const metrics: PoolMetrics[] = [];
  
  // Parse pg_pool_* metrics
  const poolTotalRegex = /pg_pool_total_connections\{pool="([^"]+)"\}\s+([\d.]+)/g;
  const poolIdleRegex = /pg_pool_idle_connections\{pool="([^"]+)"\}\s+([\d.]+)/g;
  const poolActiveRegex = /pg_pool_active_connections\{pool="([^"]+)"\}\s+([\d.]+)/g;
  const poolWaitingRegex = /pg_pool_waiting_clients\{pool="([^"]+)"\}\s+([\d.]+)/g;
  
  const poolMap = new Map<string, Partial<PoolMetrics>>();
  
  let match;
  while ((match = poolTotalRegex.exec(metricsText)) !== null) {
    const poolName = match[1];
    poolMap.set(poolName, {
      pool_name: poolName,
      total_connections: parseFloat(match[2]),
      idle_connections: 0,
      active_connections: 0,
      waiting_clients: 0,
      saturation_pct: 0,
    });
  }
  
  while ((match = poolIdleRegex.exec(metricsText)) !== null) {
    const poolName = match[1];
    if (poolMap.has(poolName)) {
      poolMap.get(poolName)!.idle_connections = parseFloat(match[2]);
    }
  }
  
  while ((match = poolActiveRegex.exec(metricsText)) !== null) {
    const poolName = match[1];
    if (poolMap.has(poolName)) {
      poolMap.get(poolName)!.active_connections = parseFloat(match[2]);
    }
  }
  
  while ((match = poolWaitingRegex.exec(metricsText)) !== null) {
    const poolName = match[1];
    if (poolMap.has(poolName)) {
      poolMap.get(poolName)!.waiting_clients = parseFloat(match[2]);
    }
  }
  
  poolMap.forEach(pool => {
    const total = pool.total_connections || 1;
    const active = pool.active_connections || 0;
    pool.saturation_pct = Math.round((active / total) * 100);
    metrics.push(pool as PoolMetrics);
  });
  
  return metrics;
}

function parseErrorMetrics(metricsText: string): ErrorMetrics[] {
  const metrics: ErrorMetrics[] = [];
  
  // Parse http_requests_total with status >= 400
  const errorRegex = /http_requests_total\{([^}]*status="([45]\d{2})"[^}]*)\}\s+([\d.]+)/g;
  
  const errorMap = new Map<string, ErrorMetrics>();
  
  let match;
  while ((match = errorRegex.exec(metricsText)) !== null) {
    const labels = parseLabels(match[1]);
    const status = match[2];
    const count = parseFloat(match[3]);
    
    const errorClass = status.startsWith('4') ? 'client_error' : 'server_error';
    const key = `${errorClass}:${labels.path || 'all'}`;
    
    if (!errorMap.has(key)) {
      errorMap.set(key, {
        error_class: errorClass,
        count,
        rate_per_min: 0, // Would need time-series data to calculate
        last_seen: new Date().toISOString(),
        sample_message: `HTTP ${status}`,
      });
    } else {
      errorMap.get(key)!.count += count;
    }
  }
  
  errorMap.forEach(e => metrics.push(e));
  return metrics;
}

function parseQueueMetrics(metricsText: string): QueueMetrics[] {
  const metrics: QueueMetrics[] = [];
  
  // Parse bullmq_* or queue_* metrics
  const queuePendingRegex = /(?:bullmq_|queue_)pending\{queue="([^"]+)"\}\s+([\d.]+)/g;
  const queueProcessingRegex = /(?:bullmq_|queue_)active\{queue="([^"]+)"\}\s+([\d.]+)/g;
  const queueFailedRegex = /(?:bullmq_|queue_)failed\{queue="([^"]+)"\}\s+([\d.]+)/g;
  
  const queueMap = new Map<string, Partial<QueueMetrics>>();
  
  let match;
  while ((match = queuePendingRegex.exec(metricsText)) !== null) {
    queueMap.set(match[1], {
      queue_name: match[1],
      pending: parseFloat(match[2]),
      processing: 0,
      failed: 0,
      throughput_per_min: 0,
      oldest_message_age_sec: 0,
    });
  }
  
  while ((match = queueProcessingRegex.exec(metricsText)) !== null) {
    if (queueMap.has(match[1])) {
      queueMap.get(match[1])!.processing = parseFloat(match[2]);
    }
  }
  
  while ((match = queueFailedRegex.exec(metricsText)) !== null) {
    if (queueMap.has(match[1])) {
      queueMap.get(match[1])!.failed = parseFloat(match[2]);
    }
  }
  
  queueMap.forEach(q => metrics.push(q as QueueMetrics));
  return metrics;
}

async function parseHealthMetrics(metricsText: string): Promise<AppTelemetry['health']> {
  // Parse process_uptime_seconds and app_version
  const uptimeMatch = metricsText.match(/process_uptime_seconds\s+([\d.]+)/);
  const versionMatch = metricsText.match(/app_info\{version="([^"]+)"/);
  const deployMatch = metricsText.match(/app_last_deploy_timestamp\s+([\d.]+)/);
  
  return {
    status: 'OK', // Would need health check endpoint for real status
    uptime_sec: uptimeMatch ? parseFloat(uptimeMatch[1]) : 0,
    version: versionMatch ? versionMatch[1] : 'unknown',
    last_deploy: deployMatch 
      ? new Date(parseFloat(deployMatch[1]) * 1000).toISOString()
      : 'unknown',
  };
}

function parseLabels(labelString: string): Record<string, string> {
  const labels: Record<string, string> = {};
  const labelRegex = /(\w+)="([^"]+)"/g;
  
  let match;
  while ((match = labelRegex.exec(labelString)) !== null) {
    labels[match[1]] = match[2];
  }
  
  return labels;
}
