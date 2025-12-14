/**
 * POSTGRES TELEMETRY ADAPTER
 * 
 * Read-only access to PostgreSQL system views for monitoring:
 * - pg_stat_statements (top queries)
 * - pg_stat_activity (current sessions)
 * - pg_stat_user_tables (table stats)
 * - WAL/IO/vacuum health indicators
 */

import { queryReadOnly } from '../database';
import { logger } from '../logger';

export interface TopQuery {
  queryid: string;
  query: string;
  calls: number;
  total_time_ms: number;
  mean_time_ms: number;
  rows: number;
  shared_blks_hit: number;
  shared_blks_read: number;
}

export interface ActiveSession {
  pid: number;
  usename: string;
  application_name: string;
  client_addr: string;
  state: string;
  query: string;
  query_start: Date;
  wait_event_type: string | null;
  wait_event: string | null;
  backend_start: Date;
  xact_start: Date | null;
}

export interface BlockingSession {
  blocked_pid: number;
  blocked_user: string;
  blocked_query: string;
  blocking_pid: number;
  blocking_user: string;
  blocking_query: string;
  blocked_since: Date;
}

export interface DatabaseStats {
  database: string;
  size_bytes: number;
  numbackends: number;
  xact_commit: number;
  xact_rollback: number;
  blks_read: number;
  blks_hit: number;
  tup_returned: number;
  tup_fetched: number;
  tup_inserted: number;
  tup_updated: number;
  tup_deleted: number;
  conflicts: number;
  deadlocks: number;
}

/**
 * Get top queries by total time from pg_stat_statements
 */
export async function getTopQueries(limit = 20): Promise<TopQuery[]> {
  try {
    const sql = `
      SELECT 
        queryid::text,
        query,
        calls,
        total_exec_time as total_time_ms,
        mean_exec_time as mean_time_ms,
        rows,
        shared_blks_hit,
        shared_blks_read
      FROM pg_stat_statements
      WHERE userid = (SELECT usesysid FROM pg_user WHERE usename = current_user)
      ORDER BY total_exec_time DESC
      LIMIT $1
    `;
    return await queryReadOnly<TopQuery>(sql, [limit]);
  } catch (err) {
    logger.warn('pg_stat_statements not available', { error: (err as Error).message });
    return [];
  }
}

/**
 * Get active database sessions
 */
export async function getActiveSessions(): Promise<ActiveSession[]> {
  const sql = `
    SELECT 
      pid,
      usename,
      application_name,
      client_addr::text,
      state,
      query,
      query_start,
      wait_event_type,
      wait_event,
      backend_start,
      xact_start
    FROM pg_stat_activity
    WHERE state IS NOT NULL
    AND pid != pg_backend_pid()
    ORDER BY query_start DESC NULLS LAST
  `;
  return await queryReadOnly<ActiveSession>(sql);
}

/**
 * Get blocking relationships (who blocks whom)
 */
export async function getBlockingSessions(): Promise<BlockingSession[]> {
  const sql = `
    SELECT 
      blocked.pid as blocked_pid,
      blocked.usename as blocked_user,
      blocked.query as blocked_query,
      blocking.pid as blocking_pid,
      blocking.usename as blocking_user,
      blocking.query as blocking_query,
      blocked.query_start as blocked_since
    FROM pg_stat_activity blocked
    JOIN pg_stat_activity blocking 
      ON blocking.pid = ANY(pg_blocking_pids(blocked.pid))
    WHERE cardinality(pg_blocking_pids(blocked.pid)) > 0
    ORDER BY blocked.query_start ASC
  `;
  return await queryReadOnly<BlockingSession>(sql);
}

/**
 * Get long-running transactions (potential issues)
 */
export async function getLongTransactions(thresholdMinutes = 5): Promise<ActiveSession[]> {
  const sql = `
    SELECT 
      pid,
      usename,
      application_name,
      client_addr::text,
      state,
      query,
      query_start,
      wait_event_type,
      wait_event,
      backend_start,
      xact_start
    FROM pg_stat_activity
    WHERE xact_start IS NOT NULL
    AND xact_start < NOW() - INTERVAL '${thresholdMinutes} minutes'
    AND state != 'idle'
    ORDER BY xact_start ASC
  `;
  return await queryReadOnly<ActiveSession>(sql);
}

/**
 * Get idle-in-transaction sessions (potential connection leaks)
 */
export async function getIdleInTransactionSessions(thresholdMinutes = 1): Promise<ActiveSession[]> {
  const sql = `
    SELECT 
      pid,
      usename,
      application_name,
      client_addr::text,
      state,
      query,
      query_start,
      wait_event_type,
      wait_event,
      backend_start,
      xact_start
    FROM pg_stat_activity
    WHERE state = 'idle in transaction'
    AND xact_start < NOW() - INTERVAL '${thresholdMinutes} minutes'
    ORDER BY xact_start ASC
  `;
  return await queryReadOnly<ActiveSession>(sql);
}

/**
 * Get database-level statistics
 */
export async function getDatabaseStats(): Promise<DatabaseStats[]> {
  const sql = `
    SELECT 
      datname as database,
      pg_database_size(datname) as size_bytes,
      numbackends,
      xact_commit,
      xact_rollback,
      blks_read,
      blks_hit,
      tup_returned,
      tup_fetched,
      tup_inserted,
      tup_updated,
      tup_deleted,
      conflicts,
      deadlocks
    FROM pg_stat_database
    WHERE datname NOT LIKE 'template%'
    ORDER BY pg_database_size(datname) DESC
  `;
  return await queryReadOnly<DatabaseStats>(sql);
}

/**
 * Get WAL statistics
 */
export async function getWALStats(): Promise<{
  wal_bytes: number;
  wal_fpi: number;
  wal_records: number;
  stats_reset: Date | null;
}> {
  try {
    const sql = `
      SELECT 
        wal_bytes,
        wal_fpi,
        wal_records,
        stats_reset
      FROM pg_stat_wal
    `;
    const rows = await queryReadOnly<any>(sql);
    return rows[0] || { wal_bytes: 0, wal_fpi: 0, wal_records: 0, stats_reset: null };
  } catch (err) {
    logger.warn('pg_stat_wal not available', { error: (err as Error).message });
    return { wal_bytes: 0, wal_fpi: 0, wal_records: 0, stats_reset: null };
  }
}

/**
 * Get vacuum and analyze stats for tables
 */
export async function getTableMaintenanceStats(): Promise<Array<{
  schemaname: string;
  relname: string;
  n_live_tup: number;
  n_dead_tup: number;
  last_vacuum: Date | null;
  last_autovacuum: Date | null;
  last_analyze: Date | null;
  last_autoanalyze: Date | null;
}>> {
  const sql = `
    SELECT 
      schemaname,
      relname,
      n_live_tup,
      n_dead_tup,
      last_vacuum,
      last_autovacuum,
      last_analyze,
      last_autoanalyze
    FROM pg_stat_user_tables
    ORDER BY n_dead_tup DESC
    LIMIT 50
  `;
  return await queryReadOnly(sql);
}

/**
 * Get replication status (if applicable)
 */
export async function getReplicationStatus(): Promise<Array<{
  client_addr: string;
  state: string;
  sent_lsn: string;
  write_lsn: string;
  flush_lsn: string;
  replay_lsn: string;
  write_lag: string | null;
  flush_lag: string | null;
  replay_lag: string | null;
}>> {
  try {
    const sql = `
      SELECT 
        client_addr::text,
        state,
        sent_lsn::text,
        write_lsn::text,
        flush_lsn::text,
        replay_lsn::text,
        write_lag::text,
        flush_lag::text,
        replay_lag::text
      FROM pg_stat_replication
    `;
    return await queryReadOnly(sql);
  } catch (err) {
    return [];
  }
}
