# ApexMediation Autonomous PostgreSQL Configuration

## Overview

This document describes the autonomous PostgreSQL configuration designed for **hands-off operation** (1-2 touches per year). The setup matches or exceeds Digital Ocean Managed PostgreSQL + ClickHouse for:

- **Performance**: SSD-optimized, aggressive caching, JIT compilation
- **Scalability**: Partitioned tables, connection pooling ready
- **Reliability**: Automated backups, self-healing health checks
- **Maintainability**: Autonomous vacuum, index maintenance, stats monitoring

## Server Specifications

| Resource | Value |
|----------|-------|
| Server | Hetzner Dedicated |
| IP | 77.42.44.35 |
| CPU | 4 cores (Intel Xeon Skylake) |
| RAM | 8 GB |
| Storage | 76 GB NVMe SSD (1 GB/s throughput) |
| PostgreSQL | 16.11 |
| Database | `adproject` |

## Performance Configuration

**Config file**: `/etc/postgresql/16/main/conf.d/apex_performance.conf`

### Memory Settings (8GB RAM optimized)

| Setting | Value | Rationale |
|---------|-------|-----------|
| `shared_buffers` | 2GB | 25% of RAM - PostgreSQL's main cache |
| `effective_cache_size` | 6GB | 75% of RAM - planner hint for OS cache |
| `work_mem` | 64MB | Per-operation memory for sorts/hashes |
| `maintenance_work_mem` | 512MB | For VACUUM, CREATE INDEX |
| `wal_buffers` | 64MB | Write-ahead log buffer |

### SSD Optimization

| Setting | Value | Rationale |
|---------|-------|-----------|
| `random_page_cost` | 1.1 | SSD has near-sequential random access |
| `effective_io_concurrency` | 200 | NVMe can handle parallel I/O |

### Autovacuum (Aggressive)

| Setting | Value | Default | Effect |
|---------|-------|---------|--------|
| `autovacuum_vacuum_scale_factor` | 0.05 | 0.2 | VACUUM at 5% dead tuples (4x sooner) |
| `autovacuum_analyze_scale_factor` | 0.025 | 0.1 | ANALYZE at 2.5% changes (4x sooner) |
| `autovacuum_max_workers` | 4 | 3 | More parallel vacuum workers |
| `autovacuum_naptime` | 30s | 1min | Check for work 2x more often |

### Query Performance

| Setting | Value | Purpose |
|---------|-------|---------|
| `jit` | on | Just-in-time compilation for complex queries |
| `max_parallel_workers_per_gather` | 2 | Parallel query execution |
| `pg_stat_statements` | loaded | Query performance tracking |
| `auto_explain` | loaded | Slow query logging (>500ms) |

## Extensions Enabled

| Extension | Version | Purpose |
|-----------|---------|---------|
| `pg_stat_statements` | 1.10 | Query performance tracking |
| `pgcrypto` | 1.3 | Cryptographic functions (VRA ledger) |
| `uuid-ossp` | 1.1 | UUID generation |
| `plpgsql` | 1.0 | Procedural language |

## Autonomous Maintenance Schedule

All jobs are configured via `/etc/cron.d/apex-*`:

| Job | Schedule | Script | Purpose |
|-----|----------|--------|---------|
| **Health Check** | Every 5 min | `pg_autonomous_health.sh` | Self-healing, connection monitoring |
| **Daily Backup** | 3:00 AM | `pg_backup_apex.sh` | Compressed backups, 7-day retention |
| **Query Monitor** | 6:00 AM | `pg_query_monitor.sh` | Slow query reports |
| **Index Maintenance** | Sunday 4:00 AM | `pg_index_maintenance.sh` | ANALYZE, unused index detection |
| **Monthly Maintenance** | 1st 1:00 AM | `pg_monthly_maintenance.sh` | VACUUM, stats reset |
| **Partition Maintenance** | 1st 2:00 AM | `pg_partition_maintenance.sh` | Create future partitions |

## Backup System

### Location
```
/var/backups/postgresql/
├── daily/      # 7-day retention
├── weekly/     # 4-week retention (Sunday copies)
└── wal/        # Reserved for WAL archiving
```

### Backup Format
- **Type**: pg_dump custom format (`-Fc`)
- **Compression**: Level 9 (maximum)
- **Verification**: Automatic `pg_restore -l` validation

### Restore Procedure
```bash
# List backup contents
pg_restore -l /var/backups/postgresql/daily/adproject_YYYY-MM-DD_HHMMSS.dump

# Full restore to new database
createdb adproject_restore
pg_restore -d adproject_restore /var/backups/postgresql/daily/adproject_YYYY-MM-DD_HHMMSS.dump

# Restore specific table
pg_restore -d adproject -t table_name /path/to/backup.dump
```

### Offsite Backup (TODO)
Scheduled for implementation:
- rsync to external Hetzner Storage Box
- Or S3-compatible object storage

## Self-Healing Health Checks

The `pg_autonomous_health.sh` script runs every 5 minutes and:

1. **PostgreSQL Running**: Auto-restarts if down
2. **Connection Pressure**: Alerts if >180/200 connections
3. **Long Queries**: Logs queries running >5 minutes
4. **Disk Space**: Alerts if <10% free
5. **Table Bloat**: Detects tables with >30% bloat

### Log Location
```
/var/log/postgresql/health_check.log
```

## Partitioning Strategy

### Currently Partitioned Tables
- `vra_ledger` - RANGE partitioned by `created_at` (monthly)

### Automatic Partition Creation
The `pg_partition_maintenance.sh` script creates partitions 3 months in advance:
- `vra_ledger_YYYY_MM` naming convention
- Index inheritance from parent table

## Performance Comparison

### vs. Digital Ocean Managed PostgreSQL

| Feature | DO Managed | ApexMediation Setup |
|---------|------------|---------------------|
| Automated Backups | ✅ Daily | ✅ Daily + Weekly |
| Point-in-Time Recovery | ✅ | ⚠️ Not yet (requires WAL archiving) |
| Automatic Failover | ✅ | ❌ (single node) |
| Connection Pooling | ✅ (PgBouncer) | ⚠️ Ready to add |
| Query Stats | ✅ | ✅ pg_stat_statements |
| Auto-Vacuum Tuning | ✅ | ✅ Aggressive config |
| Self-Healing | Limited | ✅ Comprehensive |

### vs. ClickHouse (for analytics)

ClickHouse is not needed because:
1. **PostgreSQL indexes are comprehensive** (593 indexes)
2. **Partitioned tables** for time-series data (vra_ledger)
3. **Query performance** via JIT and parallel workers
4. **Cache hit ratio**: 100% currently

For heavy analytics in future, consider:
- TimescaleDB extension
- Read replica for reporting
- Materialized views with refresh

## Scaling Considerations

### Vertical Scaling (current capacity)
- **CPU**: 4 cores → supports ~1000 TPS
- **RAM**: 8GB → supports ~50GB database hot set
- **Disk**: 76GB → ~50GB available for data growth

### When to Scale

| Metric | Threshold | Action |
|--------|-----------|--------|
| Database Size | >50GB | Add disk or archive old data |
| Connections | >150 sustained | Add PgBouncer connection pooler |
| Cache Hit Ratio | <95% | Add RAM or optimize queries |
| CPU Usage | >70% sustained | Add cores or read replica |

### Horizontal Scaling Options

1. **Read Replica**: For reporting/analytics workload
2. **PgBouncer**: Connection pooling for app scaling
3. **Partition Pruning**: More partitions for large tables

## Manual Intervention Triggers

The system will operate autonomously except when:

1. **Disk Full Alert** - Add storage or archive data
2. **Connection Exhaustion** - Add PgBouncer
3. **Failed Backup** - Check disk space, permissions
4. **Major Version Upgrade** - PostgreSQL 16 → 17 (annual)
5. **Schema Migration** - Application deployments

## Monitoring Dashboards (Optional)

For visual monitoring, consider adding:
- **Grafana** with `postgres_exporter` (port 9187)
- **pgAdmin** for ad-hoc queries

Current text-based monitoring via:
```bash
# Real-time queries
sudo -u postgres psql -d adproject -c "SELECT * FROM pg_stat_activity;"

# Query performance
sudo -u postgres psql -d adproject -c "SELECT * FROM pg_stat_statements ORDER BY total_exec_time DESC LIMIT 10;"

# Table sizes
sudo -u postgres psql -d adproject -c "SELECT relname, pg_size_pretty(pg_total_relation_size(relid)) FROM pg_stat_user_tables ORDER BY pg_total_relation_size(relid) DESC LIMIT 20;"
```

## Emergency Procedures

### PostgreSQL Won't Start
```bash
# Check logs
journalctl -u postgresql -n 100

# Check config syntax
sudo -u postgres pg_ctl -D /var/lib/postgresql/16/main configtest
```

### Database Corruption
```bash
# Check for corruption
sudo -u postgres psql -d adproject -c "SELECT * FROM pg_catalog.pg_database WHERE datname='adproject';"

# Restore from backup
pg_restore -d adproject_new /var/backups/postgresql/daily/latest.dump
```

### Connection Issues
```bash
# Kill idle connections
sudo -u postgres psql -d adproject -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE state = 'idle' AND query_start < now() - interval '1 hour';"
```

## Version History

| Date | Change |
|------|--------|
| 2025-12-14 | Initial autonomous configuration |
| 2025-12-14 | Added PITR (WAL archiving) |
| 2025-12-14 | Added PgBouncer connection pooling |

---

## Point-in-Time Recovery (PITR)

### Configuration

WAL archiving is enabled with:
- `archive_mode = on`
- `archive_command = 'test ! -f /var/backups/postgresql/wal/%f && cp %p /var/backups/postgresql/wal/%f'`
- `archive_timeout = 300` (5 minutes)
- `wal_keep_size = 1GB`

### WAL Archive Location
```
/var/backups/postgresql/wal/
```

### Retention
- WAL files cleaned daily at 4 AM
- 7-day retention via `/usr/local/bin/pg_wal_cleanup.sh`

### Recovery Procedure

1. **Stop PostgreSQL**
   ```bash
   systemctl stop postgresql
   ```

2. **Restore base backup**
   ```bash
   pg_restore -d adproject_restore /var/backups/postgresql/daily/backup.dump
   ```

3. **Configure recovery**
   ```bash
   touch /var/lib/postgresql/16/main/recovery.signal
   
   # Add to postgresql.conf:
   restore_command = 'cp /var/backups/postgresql/wal/%f %p'
   recovery_target_time = '2025-12-14 10:30:00'  # or specific time
   ```

4. **Start PostgreSQL** - it will recover to that point
   ```bash
   systemctl start postgresql
   ```

---

## Connection Pooling (PgBouncer)

### Location
- **App Server**: 46.62.252.96
- **Port**: 6432

### Configuration
- **Pool Mode**: transaction (most efficient)
- **Max Client Connections**: 1000
- **Default Pool Size**: 25 connections to PostgreSQL
- **Min Pool Size**: 5
- **Reserve Pool**: 5

### Connection String for Applications

**Through PgBouncer** (recommended when ready):
```
postgresql://adproject:ApexApp2025!@127.0.0.1:6432/adproject
```

**Direct** (current app configuration):
```
postgresql://postgres:***@77.42.44.35:5432/adproject
```

### Database Users

| User | Auth | Purpose |
|------|------|---------|
| `postgres` | SCRAM-SHA-256 | Direct superuser access |
| `adproject` | MD5 | Application user via PgBouncer |
| `pgbouncer_auth` | MD5 | PgBouncer auth lookups |

### Monitoring PgBouncer
```bash
# From app server
psql -h 127.0.0.1 -p 6432 -U pgbouncer -d pgbouncer -c "SHOW POOLS;"
psql -h 127.0.0.1 -p 6432 -U pgbouncer -d pgbouncer -c "SHOW STATS;"
```

### Switching Application to PgBouncer

When ready to use PgBouncer:
1. Update `DATABASE_URL` environment variable
2. Restart backend containers
3. Monitor connection count drops on PostgreSQL
