#!/bin/bash
set -eu
# WAL Archive Cleanup Script
# Removes WAL archives older than 7 days to prevent disk fill
# Should be run daily via cron

WAL_DIR="/var/backups/postgresql/wal"
DAYS_TO_KEEP=7

# Check if directory exists
if [ ! -d "$WAL_DIR" ]; then
    echo "WAL directory $WAL_DIR does not exist"
    exit 1
fi

# Find and delete WAL files older than DAYS_TO_KEEP
echo "Cleaning WAL archives older than $DAYS_TO_KEEP days..."
find "$WAL_DIR" -type f -mtime +$DAYS_TO_KEEP -name "0*" -delete

# Report current usage
echo "Current WAL archive usage:"
du -sh "$WAL_DIR"

echo "WAL cleanup completed successfully"
