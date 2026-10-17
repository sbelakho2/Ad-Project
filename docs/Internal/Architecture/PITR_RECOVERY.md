# PITR Recovery

> **Note:** The example commands below use RFC 5737 documentation addresses for the jump host and the DB server; replace them with your server addresses before running them.

## Overview

Our database server (203.0.113.12) has WAL archiving enabled, allowing point-in-time recovery to any moment within the configured retention window.

## Configuration

- **WAL Level**: replica
- **Archive Mode**: on
- **Archive Command**: `test ! -f /var/backups/postgresql/wal/%f && cp %p /var/backups/postgresql/wal/%f`
- **Archive Timeout**: 300 seconds (5 minutes)
- **Archive Location**: `/var/backups/postgresql/wal/`
- **Retention**: 7 days (cleaned automatically via cron)

## Current WAL Archive Size

- WAL archive size is workload-dependent (ingest-heavy days can be much larger).
- Each WAL file is 16MB.
- Cleaned daily via `/etc/cron.d/apex-postgres` → `/usr/local/bin/pg_wal_cleanup.sh`.

## Keeping PITR Viable (WAL Volume Control)

If `/var/backups/postgresql/wal/` grows too quickly to keep a 7-day window on the DB server disk:

1. Enable WAL compression (`wal_compression=zstd`).
2. Reduce checkpoint churn (increase `max_wal_size`, increase `checkpoint_timeout`).
3. Re-measure WAL archive growth after 24 hours.
4. Only then consider reducing WAL retention days as an emergency measure.

## How to Perform Point-In-Time Recovery

### Prerequisites

1. SSH access to DB server (203.0.113.12) via jump host (203.0.113.10)
2. Base backup taken with `pg_basebackup`
3. WAL archives available in `/var/backups/postgresql/wal/`
4. Target recovery time determined

### Step 1: Stop PostgreSQL

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo systemctl stop postgresql@16-main'"
```

### Step 2: Backup Current Data Directory

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres mv /var/lib/postgresql/16/main /var/lib/postgresql/16/main.backup'"
```

### Step 3: Restore Base Backup

```bash
# If using pg_basebackup restoration
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres tar -xzf /path/to/base_backup.tar.gz -C /var/lib/postgresql/16/'"
```

### Step 4: Create recovery.signal File

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres touch /var/lib/postgresql/16/main/recovery.signal'"
```

### Step 5: Configure Recovery Settings

Create `/var/lib/postgresql/16/main/postgresql.auto.conf` with recovery parameters:

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres cat > /var/lib/postgresql/16/main/postgresql.auto.conf << EOF
restore_command = '\''cp /var/backups/postgresql/wal/%f %p'\''
recovery_target_time = '\''2025-12-24 12:00:00 UTC'\''
recovery_target_action = '\''promote'\''
EOF
'"
```

**Recovery Options:**

- `recovery_target_time`: Recover to specific timestamp
- `recovery_target_name`: Recover to named restore point
- `recovery_target_xid`: Recover to specific transaction ID
- `recovery_target_lsn`: Recover to specific LSN
- `recovery_target_immediate`: Recover as soon as consistent

**Recovery Actions:**

- `pause`: Pause recovery at target (default)
- `promote`: Automatically promote server after recovery
- `shutdown`: Shutdown server after recovery

### Step 6: Start PostgreSQL and Monitor Recovery

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo systemctl start postgresql@16-main && sudo tail -f /var/log/postgresql/postgresql-16-main.log'"
```

Watch for:

- "starting point-in-time recovery to..."
- "redo starts at..."
- "recovery has paused"
- "database system is ready to accept connections"

### Step 7: Verify Recovery

```bash
ssh -i ~/.ssh/hetzner-db-mac root@203.0.113.10 "export PGPASSWORD='<REDACTED_PASSWORD>' && psql -h 203.0.113.12 -U adproject -d adproject -c 'SELECT now(), pg_postmaster_start_time(), pg_is_in_recovery();'"
```

### Step 8: Promote Server (if recovery_target_action was 'pause')

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -c \"SELECT pg_wal_replay_resume();\"'"
```

## Creating Base Backups

### Manual Base Backup

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres pg_basebackup -D /var/backups/postgresql/base/$(date +%Y%m%d_%H%M%S) -Ft -z -Xs -P'"
```

### Automated Daily Base Backup (Recommended)

Add to DB server cron:

```bash
0 1 * * * /usr/local/bin/pg-backup.sh >> /var/log/pg-backup.log 2>&1
```

Create `/usr/local/bin/pg-backup.sh`:

```bash
#!/bin/bash
BACKUP_DIR="/var/backups/postgresql/base"
DATE=$(date +%Y%m%d)
KEEP_DAYS=7

# Create backup
sudo -u postgres pg_basebackup -D "$BACKUP_DIR/$DATE" -Ft -z -Xs -P

# Clean old backups
find "$BACKUP_DIR" -maxdepth 1 -type d -mtime +$KEEP_DAYS -exec rm -rf {} \;

echo "$(date): Backup completed - $BACKUP_DIR/$DATE"
```

## Testing Recovery

### Test Recovery Process (Non-Destructive)

Steps:

- Create test restore point:

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -d adproject -c \"SELECT pg_create_restore_point('\''test_recovery_$(date +%Y%m%d)'\'');\"'"
```

- Make test changes.
- Restore to the restore point in a test environment.

### Monitoring WAL Archive Health

```bash
# Check archive_command failures
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -c \"SELECT * FROM pg_stat_archiver;\"'"

# Check WAL archive lag
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -c \"SELECT pg_walfile_name(pg_current_wal_lsn());\"'"
```

## Troubleshooting

### Archive Command Failing

```bash
# Check permissions
ls -la /var/backups/postgresql/wal/

# Check disk space
df -h /var/backups

# Review PostgreSQL logs
sudo tail -100 /var/log/postgresql/postgresql-16-main.log | grep archive
```

### Recovery Stuck

```bash
# Check recovery progress
sudo -u postgres psql -c "SELECT pg_last_wal_replay_lsn(), now() - pg_last_xact_replay_timestamp() AS replication_lag;"

# Check for missing WAL files
sudo tail -100 /var/log/postgresql/postgresql-16-main.log | grep -i "could not open file"
```

## Security Notes

- WAL archives contain all database changes and should be secured.
- Consider encrypting WAL archives if stored off-server.
- Restrict access to `/var/backups/postgresql/` to `postgres` user only.

## Related Maintenance

- **Disk Space Monitoring**: `/usr/local/bin/check-disk-space.sh` (hourly; via `/etc/cron.d/apex-postgres`)
- **WAL Cleanup**: `/usr/local/bin/pg_wal_cleanup.sh` (daily; via `/etc/cron.d/apex-postgres`)
- **PostgreSQL Log Rotation**: `/etc/logrotate.d/postgresql-custom` (daily)
