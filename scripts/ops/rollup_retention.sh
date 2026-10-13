#!/usr/bin/env bash
# Rollup partition retention script.
# Drops monthly partitions for rollup tables older than the specified retention.
#
# Usage:
#   ./scripts/ops/rollup_retention.sh [retention_days] [dry_run]
#
# Defaults:
#   retention_days=1825 (5 years)
#   dry_run=false

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

RETENTION_DAYS=${1:-1825}
DRY_RUN=${2:-false}

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

log "Starting rollup partition retention cleanup (retention=${RETENTION_DAYS} days, dry_run=${DRY_RUN})"

CUTOFF_DATE=$(date -d "-${RETENTION_DAYS} days" '+%Y-%m-%d' 2>/dev/null || date -v-${RETENTION_DAYS}d '+%Y-%m-%d')
CUTOFF_MONTH=${CUTOFF_DATE:0:7}
log "Cutoff month: ${CUTOFF_MONTH} (partitions before this month will be dropped)"

# Rollup partition naming scheme created by create_future_rollup_partitions.sh:
#   analytics_*_rollups_YYYY_MM
PARTITIONS_SQL="
SELECT
  tablename,
  regexp_replace(tablename, '.*_(\\d{4})_(\\d{2})$', '\\1-\\2') AS bucket_month,
  pg_size_pretty(pg_total_relation_size('public.' || tablename)) AS size,
  pg_total_relation_size('public.' || tablename) AS size_bytes
FROM pg_tables
WHERE schemaname = 'public'
  AND tablename ~ '^analytics_.*_rollups_\\d{4}_\\d{2}$'
  AND regexp_replace(tablename, '.*_(\\d{4})_(\\d{2})$', '\\1-\\2') < '${CUTOFF_MONTH}'
ORDER BY tablename;
"

PARTITIONS=$(sudo -u postgres psql -d adproject -t -A -F '|' -c "${PARTITIONS_SQL}" 2>/dev/null || echo "")

if [[ -z "$PARTITIONS" ]]; then
  log "No rollup partitions found older than ${CUTOFF_MONTH}. Nothing to clean up."
  exit 0
fi

log "Found rollup partitions to drop:" 
TOTAL_BYTES=0

while IFS='|' read -r tablename bucket_month size size_bytes; do
  [[ -z "$tablename" ]] && continue
  log "  - ${tablename} (${bucket_month}) size=${size}"
  TOTAL_BYTES=$((TOTAL_BYTES + size_bytes))

done <<< "$PARTITIONS"

log "Total size to reclaim (approx): $(numfmt --to=iec --suffix=B ${TOTAL_BYTES} 2>/dev/null || echo ${TOTAL_BYTES})"

if [[ "$DRY_RUN" == "true" ]]; then
  log "Dry run enabled; not dropping partitions."
  exit 0
fi

while IFS='|' read -r tablename _bucket_month _size _size_bytes; do
  [[ -z "$tablename" ]] && continue
  log "Dropping partition: ${tablename}"
  sudo -u postgres psql -d adproject -c "DROP TABLE IF EXISTS public.${tablename};" 2>/dev/null || log "Warning: failed to drop ${tablename}"

done <<< "$PARTITIONS"

log "Rollup partition retention cleanup complete."
