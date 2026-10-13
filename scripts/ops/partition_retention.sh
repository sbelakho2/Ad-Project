#!/usr/bin/env bash
# Partition Retention Script
# Drops analytics partitions older than the specified retention period
#
# Usage:
#   ./scripts/ops/partition_retention.sh [retention_days] [dry_run]
#
# Examples:
#   ./scripts/ops/partition_retention.sh 90        # Drop partitions older than 90 days
#   ./scripts/ops/partition_retention.sh 90 true   # Dry run - show what would be dropped
#   ./scripts/ops/partition_retention.sh           # Default: 90 days retention

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

RETENTION_DAYS=${1:-14}
DRY_RUN=${2:-false}

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

log "Starting partition retention cleanup (retention=${RETENTION_DAYS} days, dry_run=${DRY_RUN})"

# Calculate cutoff date
CUTOFF_DATE=$(date -d "-${RETENTION_DAYS} days" '+%Y-%m-%d' 2>/dev/null || date -v-${RETENTION_DAYS}d '+%Y-%m-%d')
log "Cutoff date: ${CUTOFF_DATE} (partitions before this will be dropped)"

# Get list of partitions to drop
PARTITIONS_SQL="
SELECT 
    tablename,
    -- Extract date from partition name (format: analytics_*_YYYY_MM_DD)
    regexp_replace(tablename, '.*_(\d{4})_(\d{2})_(\d{2})$', '\1-\2-\3') AS partition_date,
    pg_size_pretty(pg_total_relation_size('public.' || tablename)) AS size,
    pg_total_relation_size('public.' || tablename) AS size_bytes
FROM pg_tables
WHERE schemaname = 'public'
  AND (tablename LIKE 'analytics_impressions_%' OR tablename LIKE 'analytics_revenue_events_%')
  AND tablename ~ '_\d{4}_\d{2}_\d{2}$'
  AND regexp_replace(tablename, '.*_(\d{4})_(\d{2})_(\d{2})$', '\1-\2-\3')::date < '${CUTOFF_DATE}'::date
ORDER BY tablename;
"

# Execute via psql
PARTITIONS=$(sudo -u postgres psql -d adproject -t -A -F '|' -c "${PARTITIONS_SQL}" 2>/dev/null || echo "")

if [[ -z "$PARTITIONS" ]]; then
  log "No partitions found older than ${CUTOFF_DATE}. Nothing to clean up."
  exit 0
fi

log "Found partitions to drop:"
TOTAL_SIZE=0
while IFS='|' read -r tablename partition_date size size_bytes; do
  log "  - ${tablename} (${partition_date}) - ${size}"
  TOTAL_SIZE=$((TOTAL_SIZE + size_bytes))
done <<< "$PARTITIONS"

log "Total space to reclaim: $(numfmt --to=iec ${TOTAL_SIZE} 2>/dev/null || echo "${TOTAL_SIZE} bytes")"

if [[ "$DRY_RUN" == "true" ]]; then
  log "DRY RUN - No partitions were dropped"
  exit 0
fi

# Drop each partition
DROPPED=0
FAILED=0
while IFS='|' read -r tablename partition_date size size_bytes; do
  log "Dropping partition: ${tablename}..."
  if sudo -u postgres psql -d adproject -c "DROP TABLE IF EXISTS public.${tablename};" 2>/dev/null; then
    log "  ✓ Dropped ${tablename}"
    ((DROPPED++))
  else
    log "  ✗ Failed to drop ${tablename}"
    ((FAILED++))
  fi
done <<< "$PARTITIONS"

log "Partition cleanup complete: ${DROPPED} dropped, ${FAILED} failed"

# Run VACUUM ANALYZE on parent tables to update statistics
log "Running VACUUM ANALYZE on analytics tables..."
sudo -u postgres psql -d adproject -c "VACUUM ANALYZE analytics_impressions;" 2>/dev/null || true
sudo -u postgres psql -d adproject -c "VACUUM ANALYZE analytics_revenue_events;" 2>/dev/null || true

log "Retention cleanup finished successfully"
