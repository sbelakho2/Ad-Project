/**
 * DATABASE CONNECTIONS
 * 
 * Three separate connection pools with different privilege levels:
 * 1. readOnly - for monitoring/telemetry (pg_stat_*, read-only queries)
 * 2. operator - for safe actions (partition management, job control)
 * 3. breakGlass - for emergency actions (session termination, forced ops)
 */

import { Pool, PoolClient } from 'pg';
import { getReadOnlyDbConfig, getOperatorDbConfig, getBreakGlassDbConfig } from './config';
import { logger } from './logger';

// Lazy initialization - don't connect until needed
let readOnlyPool: Pool | null = null;
let operatorPool: Pool | null = null;
let breakGlassPool: Pool | null = null;

/**
 * Get read-only database connection pool
 * Used for: monitoring, telemetry, dashboards
 */
export function getReadOnlyPool(): Pool {
  if (!readOnlyPool) {
    const config = getReadOnlyDbConfig();
    readOnlyPool = new Pool({
      ...config,
      max: 5,
      idleTimeoutMillis: 30000,
      connectionTimeoutMillis: 5000,
    });
    
    readOnlyPool.on('error', (err) => {
      logger.error('Read-only pool error', { error: err.message });
    });
    
    logger.info('Read-only database pool initialized', { user: config.user });
  }
  return readOnlyPool;
}

/**
 * Get operator database connection pool
 * Used for: partition management, job control, safe actions
 */
export function getOperatorPool(): Pool {
  if (!operatorPool) {
    const config = getOperatorDbConfig();
    operatorPool = new Pool({
      ...config,
      max: 3,
      idleTimeoutMillis: 30000,
      connectionTimeoutMillis: 5000,
    });
    
    operatorPool.on('error', (err) => {
      logger.error('Operator pool error', { error: err.message });
    });
    
    logger.info('Operator database pool initialized', { user: config.user });
  }
  return operatorPool;
}

/**
 * Get break-glass database connection pool
 * Used for: emergency actions only (session termination, forced ops)
 * HIGHLY RESTRICTED - requires explicit justification
 */
export function getBreakGlassPool(): Pool {
  if (!breakGlassPool) {
    const config = getBreakGlassDbConfig();
    breakGlassPool = new Pool({
      ...config,
      max: 1, // Only one connection - intentionally limited
      idleTimeoutMillis: 10000,
      connectionTimeoutMillis: 5000,
    });
    
    breakGlassPool.on('error', (err) => {
      logger.error('Break-glass pool error', { error: err.message });
    });
    
    logger.warn('Break-glass database pool initialized - use with extreme caution', { 
      user: config.user 
    });
  }
  return breakGlassPool;
}

/**
 * Execute a read-only query
 */
export async function queryReadOnly<T = unknown>(
  sql: string,
  params: unknown[] = []
): Promise<T[]> {
  const pool = getReadOnlyPool();
  const result = await pool.query(sql, params);
  return result.rows as T[];
}

/**
 * Execute an operator-level query (with audit context)
 * When called with just sql and params, uses 'system' as actor
 */
export async function queryOperator<T = unknown>(
  sql: string,
  params: unknown[] = [],
  actor?: string,
  reason?: string
): Promise<T[]> {
  const pool = getOperatorPool();
  const effectiveActor = actor || 'system';
  const effectiveReason = reason || 'autonomous-operation';
  logger.info('Operator query', { actor: effectiveActor, reason: effectiveReason, sql: sql.substring(0, 100) });
  const result = await pool.query(sql, params);
  return result.rows as T[];
}

/**
 * Execute a break-glass query (requires explicit justification)
 */
export async function queryBreakGlass<T = unknown>(
  sql: string,
  params: unknown[] = [],
  actor: string,
  justification: string
): Promise<T[]> {
  const pool = getBreakGlassPool();
  logger.warn('BREAK-GLASS QUERY', { actor, justification, sql: sql.substring(0, 100) });
  const result = await pool.query(sql, params);
  return result.rows as T[];
}

/**
 * Gracefully close all pools
 */
export async function closeAllPools(): Promise<void> {
  const pools = [readOnlyPool, operatorPool, breakGlassPool].filter(Boolean) as Pool[];
  await Promise.all(pools.map(pool => pool.end()));
  readOnlyPool = null;
  operatorPool = null;
  breakGlassPool = null;
  logger.info('All database pools closed');
}

// Aliases for index.ts compatibility
export const initializePools = async () => {
  // Initialize read-only pool on startup (required)
  getReadOnlyPool();
  // Operator and break-glass pools initialize lazily
  logger.info('Database pools initialized');
};

export const closePools = closeAllPools;
