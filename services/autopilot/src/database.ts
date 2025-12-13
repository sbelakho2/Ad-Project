import { Pool, PoolClient } from 'pg';
import pino from 'pino';
import { config } from './config';

const logger = pino({ level: config.logging.level });

// Database connection pool
const pool = new Pool({
  connectionString: config.database.url,
  max: config.database.poolSize,
  idleTimeoutMillis: 30000,
  connectionTimeoutMillis: 10000,
});

pool.on('error', (err) => {
  logger.error({ err }, 'Unexpected database pool error');
});

// Health state enum
export type HealthState = 'OK' | 'DEGRADED' | 'FAIL';

// Health check result interface
export interface HealthCheckResult {
  state: HealthState;
  checks: Record<string, {
    status: 'OK' | 'WARN' | 'FAIL';
    [key: string]: unknown;
  }>;
  timestamp: Date;
}

// Autopilot job interface
export interface AutopilotJob {
  id: string;
  job_type: string;
  status: 'pending' | 'running' | 'completed' | 'failed' | 'skipped';
  scheduled_at: Date;
  started_at?: Date;
  completed_at?: Date;
  duration_ms?: number;
  result?: Record<string, unknown>;
  error?: string;
}

// Database metrics interface
export interface DatabaseMetrics {
  connections: {
    active: number;
    idle: number;
    total: number;
    max: number;
    usage_pct: number;
  };
  tables: {
    schema: string;
    name: string;
    row_count: number;
    size_bytes: number;
    dead_tuples: number;
  }[];
  partitions: {
    table: string;
    partition_count: number;
    oldest_partition: string;
    newest_partition: string;
  }[];
  performance: {
    cache_hit_ratio: number;
    transactions_per_sec: number;
    tuples_returned_per_sec: number;
  };
}

/**
 * Execute a query with automatic connection management
 */
export async function query<T = unknown>(text: string, params?: unknown[]): Promise<T[]> {
  const client = await pool.connect();
  try {
    const result = await client.query(text, params);
    return result.rows as T[];
  } finally {
    client.release();
  }
}

/**
 * Execute a query that returns a single row
 */
export async function queryOne<T = unknown>(text: string, params?: unknown[]): Promise<T | null> {
  const rows = await query<T>(text, params);
  return rows.length > 0 ? rows[0] : null;
}

/**
 * Run the database health check function
 */
export async function runHealthCheck(): Promise<HealthCheckResult> {
  logger.info('Running health check');
  
  const result = await queryOne<{ run_health_check: HealthCheckResult }>(
    'SELECT ops.run_health_check() as run_health_check'
  );
  
  if (!result) {
    throw new Error('Health check function returned no result');
  }
  
  const healthResult = result.run_health_check;
  logger.info({ state: healthResult.state }, 'Health check completed');
  
  return healthResult;
}

/**
 * Get current autopilot status
 */
export async function getAutopilotStatus(): Promise<{
  state: HealthState;
  last_check: Date;
  checks: Record<string, unknown>;
  metrics: Record<string, unknown>;
}> {
  const result = await queryOne<{
    state: HealthState;
    last_check: Date;
    checks: Record<string, unknown>;
    metrics: Record<string, unknown>;
  }>('SELECT state, last_check, checks, metrics FROM ops.autopilot_status WHERE id = 1');
  
  if (!result) {
    throw new Error('Autopilot status not found');
  }
  
  return result;
}

/**
 * Ensure future partitions exist
 */
export async function ensureFuturePartitions(): Promise<void> {
  logger.info('Ensuring future partitions');
  await query('SELECT ops.ensure_future_partitions()');
  logger.info('Future partitions ensured');
}

/**
 * Apply retention policy
 */
export async function applyRetentionPolicy(): Promise<Record<string, number>> {
  logger.info('Applying retention policy');
  
  const result = await queryOne<{ apply_retention_policy: Record<string, number> }>(
    'SELECT ops.apply_retention_policy() as apply_retention_policy'
  );
  
  if (!result) {
    throw new Error('Retention policy function returned no result');
  }
  
  logger.info({ result: result.apply_retention_policy }, 'Retention policy applied');
  return result.apply_retention_policy;
}

/**
 * Compute hourly metrics aggregation
 */
export async function computeHourlyMetrics(hour?: Date): Promise<number> {
  const targetHour = hour || new Date(Date.now() - 3600000); // Default to previous hour
  targetHour.setMinutes(0, 0, 0);
  
  logger.info({ hour: targetHour }, 'Computing hourly metrics');
  
  const result = await queryOne<{ compute_hourly_metrics: number }>(
    'SELECT agg.compute_hourly_metrics($1) as compute_hourly_metrics',
    [targetHour]
  );
  
  if (!result) {
    return 0;
  }
  
  logger.info({ count: result.compute_hourly_metrics, hour: targetHour }, 'Hourly metrics computed');
  return result.compute_hourly_metrics;
}

/**
 * Compute daily metrics aggregation
 */
export async function computeDailyMetrics(date?: Date): Promise<number> {
  const targetDate = date || new Date(Date.now() - 86400000); // Default to yesterday
  const dateStr = targetDate.toISOString().split('T')[0];
  
  logger.info({ date: dateStr }, 'Computing daily metrics');
  
  const result = await queryOne<{ compute_daily_metrics: number }>(
    'SELECT agg.compute_daily_metrics($1::date) as compute_daily_metrics',
    [dateStr]
  );
  
  if (!result) {
    return 0;
  }
  
  logger.info({ count: result.compute_daily_metrics, date: dateStr }, 'Daily metrics computed');
  return result.compute_daily_metrics;
}

/**
 * Get database metrics
 */
export async function getDatabaseMetrics(): Promise<DatabaseMetrics> {
  // Connection stats
  const connStats = await queryOne<{
    active: number;
    idle: number;
    total: number;
  }>(`
    SELECT 
      COUNT(*) FILTER (WHERE state = 'active') as active,
      COUNT(*) FILTER (WHERE state = 'idle') as idle,
      COUNT(*) as total
    FROM pg_stat_activity
    WHERE datname = current_database()
  `);
  
  const maxConnections = await queryOne<{ max: string }>(
    "SELECT current_setting('max_connections') as max"
  );
  
  // Table stats
  const tableStats = await query<{
    schema: string;
    name: string;
    row_count: number;
    size_bytes: number;
    dead_tuples: number;
  }>(`
    SELECT 
      schemaname as schema,
      relname as name,
      n_live_tup as row_count,
      pg_total_relation_size(schemaname || '.' || relname) as size_bytes,
      n_dead_tup as dead_tuples
    FROM pg_stat_user_tables
    WHERE schemaname IN ('core', 'config', 'ops', 'events', 'agg')
    ORDER BY pg_total_relation_size(schemaname || '.' || relname) DESC
    LIMIT 20
  `);
  
  // Partition stats
  const partitionStats = await query<{
    table: string;
    partition_count: number;
    oldest_partition: string;
    newest_partition: string;
  }>(`
    SELECT 
      parent.relname as table,
      COUNT(child.relname) as partition_count,
      MIN(child.relname) as oldest_partition,
      MAX(child.relname) as newest_partition
    FROM pg_inherits
    JOIN pg_class parent ON pg_inherits.inhparent = parent.oid
    JOIN pg_class child ON pg_inherits.inhrelid = child.oid
    JOIN pg_namespace ns ON parent.relnamespace = ns.oid
    WHERE ns.nspname IN ('events', 'ops', 'core')
    GROUP BY parent.relname
  `);
  
  // Performance stats
  const perfStats = await queryOne<{
    cache_hit_ratio: number;
    xact_commit: number;
    tup_returned: number;
  }>(`
    SELECT 
      CASE WHEN blks_hit + blks_read > 0 
        THEN blks_hit::numeric / (blks_hit + blks_read) * 100 
        ELSE 100 
      END as cache_hit_ratio,
      xact_commit,
      tup_returned
    FROM pg_stat_database
    WHERE datname = current_database()
  `);
  
  const max = parseInt(maxConnections?.max || '100', 10);
  const total = connStats?.total || 0;
  
  return {
    connections: {
      active: connStats?.active || 0,
      idle: connStats?.idle || 0,
      total,
      max,
      usage_pct: (total / max) * 100,
    },
    tables: tableStats,
    partitions: partitionStats,
    performance: {
      cache_hit_ratio: perfStats?.cache_hit_ratio || 0,
      transactions_per_sec: 0, // Would need delta calculation
      tuples_returned_per_sec: 0,
    },
  };
}

/**
 * Log autopilot job
 */
export async function logAutopilotJob(
  jobType: string,
  status: AutopilotJob['status'],
  result?: Record<string, unknown>,
  error?: string,
  startedAt?: Date,
  completedAt?: Date
): Promise<void> {
  const durationMs = startedAt && completedAt
    ? completedAt.getTime() - startedAt.getTime()
    : undefined;
  
  await query(`
    INSERT INTO ops.autopilot_jobs (job_type, status, scheduled_at, started_at, completed_at, duration_ms, result, error)
    VALUES ($1, $2, NOW(), $3, $4, $5, $6, $7)
  `, [jobType, status, startedAt, completedAt, durationMs, result ? JSON.stringify(result) : null, error]);
}

/**
 * Get recent autopilot jobs
 */
export async function getRecentJobs(limit = 50): Promise<AutopilotJob[]> {
  return query<AutopilotJob>(`
    SELECT id, job_type, status, scheduled_at, started_at, completed_at, duration_ms, result, error
    FROM ops.autopilot_jobs
    ORDER BY created_at DESC
    LIMIT $1
  `, [limit]);
}

/**
 * Clean up completed jobs older than N days
 */
export async function cleanupOldJobs(daysToKeep = 30): Promise<number> {
  const result = await query<{ count: number }>(`
    WITH deleted AS (
      DELETE FROM ops.autopilot_jobs
      WHERE created_at < NOW() - INTERVAL '${daysToKeep} days'
      RETURNING 1
    )
    SELECT COUNT(*) as count FROM deleted
  `);
  
  return result[0]?.count || 0;
}

/**
 * Close database pool
 */
export async function closePool(): Promise<void> {
  await pool.end();
}
