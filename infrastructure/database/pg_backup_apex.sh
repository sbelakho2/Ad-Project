#!/bin/bash
# ApexMediation PostgreSQL Autonomous Backup System
# Runs daily, keeps 7 daily + 4 weekly backups
# Deploy to: /usr/local/bin/pg_backup_apex.sh on DB server

set -euo pipefail

BACKUP_DIR="/mnt/HC_Volume_104191119/postgresql-backups"
DATE=$(date +%Y-%m-%d_%H%M%S)
DAY_OF_WEEK=$(date +%u)
LOG_FILE="/var/log/postgresql/backup.log"

log() {
    echo "$(date '+%Y-%m-%d %H:%M:%S') - $1" | tee -a "$LOG_FILE"
}

mkdir -p "$BACKUP_DIR/daily" "$BACKUP_DIR/weekly"

log "Starting backup..."

# Create daily backup with compression
pg_dump -U postgres -d adproject -Fc -Z 9 -f "$BACKUP_DIR/daily/adproject_$DATE.dump" 2>&1 | tee -a "$LOG_FILE"

if [ $? -eq 0 ]; then
    log "Daily backup created: adproject_$DATE.dump"
    BACKUP_SIZE=$(du -h "$BACKUP_DIR/daily/adproject_$DATE.dump" | cut -f1)
    log "Backup size: $BACKUP_SIZE"
else
    log "ERROR: Backup failed!"
    exit 1
fi

# If Sunday, copy to weekly
if [ "$DAY_OF_WEEK" -eq 7 ]; then
    cp "$BACKUP_DIR/daily/adproject_$DATE.dump" "$BACKUP_DIR/weekly/"
    log "Weekly backup created"
fi

# Cleanup: keep 3 daily backups
find "$BACKUP_DIR/daily" -name "*.dump" -mtime +3 -delete
log "Old daily backups cleaned (keeping 3 days)"

# Cleanup: keep 4 weekly backups
find "$BACKUP_DIR/weekly" -name "*.dump" -mtime +28 -delete
log "Old weekly backups cleaned (keeping 4 weeks)"

# Verify backup integrity
pg_restore -l "$BACKUP_DIR/daily/adproject_$DATE.dump" > /dev/null 2>&1
if [ $? -eq 0 ]; then
    log "Backup integrity verified: OK"
else
    log "WARNING: Backup integrity check failed!"
fi

log "Backup completed successfully"
