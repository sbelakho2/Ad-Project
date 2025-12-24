-- Database Health Check Script
-- Run this regularly to monitor database health

-- 1. Check database size
SELECT 
    pg_database.datname AS database,
    pg_size_pretty(pg_database_size(pg_database.datname)) AS size
FROM pg_database
WHERE datname = current_database();

-- 2. Check table sizes (top 10)
SELECT 
    schemaname,
    relname AS table_name,
    pg_size_pretty(pg_total_relation_size(relid)) AS total_size,
    pg_size_pretty(pg_relation_size(relid)) AS table_size,
    pg_size_pretty(pg_total_relation_size(relid) - pg_relation_size(relid)) AS indexes_size
FROM pg_catalog.pg_statio_user_tables
ORDER BY pg_total_relation_size(relid) DESC
LIMIT 10;

-- 3. Check for bloat (dead tuples)
SELECT 
    schemaname,
    relname,
    n_live_tup AS live_tuples,
    n_dead_tup AS dead_tuples,
    ROUND(100.0 * n_dead_tup / NULLIF(n_live_tup + n_dead_tup, 0), 2) AS dead_tuple_percent,
    last_vacuum,
    last_autovacuum
FROM pg_stat_user_tables
WHERE n_dead_tup > 1000
ORDER BY n_dead_tup DESC
LIMIT 10;

-- 4. Check index usage
SELECT 
    schemaname,
    tablename,
    indexname,
    idx_scan AS index_scans,
    idx_tup_read AS tuples_read,
    idx_tup_fetch AS tuples_fetched,
    pg_size_pretty(pg_relation_size(indexrelid)) AS index_size
FROM pg_stat_user_indexes
WHERE idx_scan = 0
AND pg_relation_size(indexrelid) > 1048576  -- Only show indexes > 1MB
ORDER BY pg_relation_size(indexrelid) DESC
LIMIT 10;

-- 5. Check long-running queries
SELECT 
    pid,
    now() - query_start AS duration,
    state,
    query
FROM pg_stat_activity
WHERE state != 'idle'
AND now() - query_start > interval '1 minute'
ORDER BY duration DESC;

-- 6. Check connection count
SELECT 
    datname AS database,
    COUNT(*) AS connections
FROM pg_stat_activity
GROUP BY datname
ORDER BY connections DESC;

-- 7. Check cache hit ratio (should be > 99%)
SELECT 
    'Buffer Cache Hit Ratio' AS metric,
    ROUND(100.0 * sum(blks_hit) / NULLIF(sum(blks_hit) + sum(blks_read), 0), 2) AS percentage
FROM pg_stat_database
WHERE datname = current_database();

-- 8. Check for replication lag (if applicable)
SELECT 
    client_addr,
    state,
    sync_state,
    pg_wal_lsn_diff(pg_current_wal_lsn(), sent_lsn) AS send_lag_bytes,
    pg_wal_lsn_diff(pg_current_wal_lsn(), write_lsn) AS write_lag_bytes,
    pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn) AS replay_lag_bytes
FROM pg_stat_replication;

-- 9. Check autovacuum activity
SELECT 
    relname,
    last_vacuum,
    last_autovacuum,
    vacuum_count,
    autovacuum_count
FROM pg_stat_user_tables
WHERE autovacuum_count > 0 OR vacuum_count > 0
ORDER BY last_autovacuum DESC NULLS LAST
LIMIT 10;

-- 10. Check disk space from PostgreSQL perspective
SELECT 
    'Database Cluster Size' AS metric,
    pg_size_pretty(sum(pg_database_size(datname))) AS size
FROM pg_database;
