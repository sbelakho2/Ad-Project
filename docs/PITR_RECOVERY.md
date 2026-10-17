# PostgreSQL Point-In-Time Recovery (PITR) Guide

See also: [docs/Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md](Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md)

> **Note:** Example commands below use RFC 5737 documentation addresses for the App Server and the DB Server; replace them with your server addresses before running them.

## Overview

Our database server (203.0.113.12) has WAL archiving enabled, allowing point-in-time recovery (PITR) to any moment within the effective retention window.

Production goal (as of 2026-01-03):

- Keep ~30 hours worth of archived WAL (>=24h PITR window)
- Create daily physical base backups

## Configuration

- **WAL Level**: replica
- **Archive Mode**: on
- **Archive Command**: `cp %p /var/backups/postgresql/wal/%f`
- **Archive Timeout**: 1 hour
- **Archive Location**: `/var/backups/postgresql/wal/`
- **Retention**: ~30 hours (enforced via cron; verify effective retention)

## Current WAL Archive Size

- Each WAL file is 16MB
- Size varies with write volume (can be small on low-traffic systems)
- Pruned automatically to maintain the configured retention window

## What Is Stored Where (Production)

- **Archived WAL**: `/var/backups/postgresql/wal/`
- **Physical base backups** (tarballs): `/var/backups/postgresql/base/base_YYYY-MM-DD.tar.gz`

Base backups and WAL retention are installed/managed by the repo script:

- `scripts/ops/hetzner_enable_pitr.sh`

On the DB server, the installed helpers live at:

- `/opt/adproject/ops/pitr_basebackup.sh`
- `/opt/adproject/ops/prune_wal_archives.sh`

And are scheduled by cron:

- `/etc/cron.d/adproject-pitr`
- `/etc/cron.d/adproject-wal-retention`

Important: legacy automation may also exist and prune WAL archives; ensure it does not shorten the PITR window.

## Verify The Effective PITR Window (Do This Before/After Any Restore)

Run on the DB server (via jump host):

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'bash -lc \"set -euo pipefail; \
echo \"== cron files ==\"; \
ls -la /etc/cron.d/adproject-pitr /etc/cron.d/adproject-wal-retention 2>/dev/null || true; \
ls -la /etc/cron.d/apex-postgres-backup-retention 2>/dev/null || true; \
echo; \
echo \"== WAL oldest/newest timestamps ==\"; \
if [[ -d /var/backups/postgresql/wal ]]; then \
  find /var/backups/postgresql/wal -type f -name \"0000*\" -printf \"%TY-%Tm-%TdT%TH:%TM:%TSZ %p\\n\" 2>/dev/null | sort | { head -n 1; tail -n 1; } || true; \
fi; \
echo; \
echo \"== base backups ==\"; \
ls -1t /var/backups/postgresql/base/base_*.tar.gz 2>/dev/null | head -n 10 || true\"'"
```

If you see multiple retention jobs (e.g. `adproject-wal-retention` plus a legacy job), the effective PITR window is the *minimum* of their prune windows.

## How to Perform Point-In-Time Recovery

### Prerequisites

1. SSH access to DB server (203.0.113.12) via jump host (203.0.113.10)
2. A physical base backup tarball available on the DB server
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
# Restore from the production-style tarball base backup:
#   /var/backups/postgresql/base/base_YYYY-MM-DD.tar.gz
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'bash -lc \"set -euo pipefail; \
sudo -u postgres mkdir -p /var/lib/postgresql/16/main; \
sudo -u postgres tar -xzf /var/backups/postgresql/base/base_YYYY-MM-DD.tar.gz -C /var/lib/postgresql/16/main\"'"
```

### Step 4: Create recovery.signal File

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres touch /var/lib/postgresql/16/main/recovery.signal'"
```

### Step 5: Configure Recovery Settings

Create a temporary recovery config under `/etc/postgresql/16/main/conf.d/` with recovery parameters.

After recovery completes, remove this file to avoid unexpected behavior on the next restart.

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo tee /etc/postgresql/16/main/conf.d/90-pitr-recovery.conf >/dev/null << EOF
# Temporary PITR recovery settings (remove after restore)
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
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -d adproject -c \"SELECT now(), pg_postmaster_start_time(), pg_is_in_recovery();\"'"
```

### Step 8: Promote Server (if recovery_target_action was 'pause')

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -c \"SELECT pg_promote(true, 60);\"'"
```

### Step 9: Cleanup Temporary Recovery Config

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo rm -f /etc/postgresql/16/main/conf.d/90-pitr-recovery.conf'"
```

## Creating Base Backups

### Manual Base Backup

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres /opt/adproject/ops/pitr_basebackup.sh'"
```

### Automated Daily Base Backup (Recommended)

Production uses cron-managed automation installed by `scripts/ops/hetzner_enable_pitr.sh`.

Check on DB host:

```bash
ls -la /etc/cron.d/adproject-pitr
sudo tail -n 200 /var/log/adproject-pitr.log
ls -1t /var/backups/postgresql/base/base_*.tar.gz | head
```

## Testing Recovery

### Test Recovery Process (Non-Destructive)

1. Create test restore point:

```bash
ssh -i ~/.ssh/hetzner-db-mac -A root@203.0.113.10 "ssh root@203.0.113.12 'sudo -u postgres psql -d adproject -c \"SELECT pg_create_restore_point('\''test_recovery_$(date +%Y%m%d)'\'');\"'"
```

1. Make test changes
1. Restore to restore point in test environment

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

- WAL archives contain all database changes and should be secured
- Consider encrypting WAL archives if stored off-server
- Restrict access to `/var/backups/postgresql/` to postgres user only
- Monitor archive directory disk usage and verify retention job(s)

## Related Maintenance

- **Disk Space Monitoring**: `/usr/local/bin/check-disk-space.sh` (daily 2 AM)
- **PostgreSQL Log Rotation**: `/etc/logrotate.d/postgresql-custom` (daily)

Production PITR automation:

- **Base backups**: `/opt/adproject/ops/pitr_basebackup.sh` (scheduled by `/etc/cron.d/adproject-pitr`)
- **WAL retention**: `/opt/adproject/ops/prune_wal_archives.sh` (scheduled by `/etc/cron.d/adproject-wal-retention`)
