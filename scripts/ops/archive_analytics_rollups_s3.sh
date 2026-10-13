#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

RETENTION_MONTHS=${1:-3}
DRY_RUN=${2:-false}

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

if [[ -f /opt/adproject/ops/s3_archive.env ]]; then
  set -a
  # shellcheck disable=SC1091
  source /opt/adproject/ops/s3_archive.env
  set +a
fi

S3_BUCKET="${S3_ANALYTICS_BUCKET:-${S3_BUCKET:-}}"
S3_PREFIX="${S3_ANALYTICS_PREFIX:-analytics/rollups/}"
BACKUP_DIR="${ANALYTICS_BACKUP_DIR:-/mnt/HC_Volume_104191119/postgresql-backups/analytics-rollups}"

if [[ -z "${S3_BUCKET:-}" || -z "${AWS_ACCESS_KEY_ID:-}" || -z "${AWS_SECRET_ACCESS_KEY:-}" ]]; then
  echo "[ERROR] Missing S3 credentials/env; refusing to archive rollups." >&2
  exit 1
fi

mkdir -p "$BACKUP_DIR"

CUTOFF_DATE=$(date -d "$(date +%Y-%m-01) -${RETENTION_MONTHS} months" '+%Y-%m-01' 2>/dev/null || date -v-${RETENTION_MONTHS}m -v1d '+%Y-%m-01')
log "Archiving analytics rollups older than ${CUTOFF_DATE} (retention=${RETENTION_MONTHS} months, dry_run=${DRY_RUN})"

PARTITIONS_SQL="
SELECT
  tablename,
  regexp_replace(tablename, '.*_(\\d{4})_(\\d{2})$', '\\1-\\2-01') AS partition_month,
  pg_total_relation_size('public.' || tablename) AS size_bytes
FROM pg_tables
WHERE schemaname = 'public'
  AND tablename ~ '^analytics_(metrics|revenue)_rollups_\\d{4}_\\d{2}$'
  AND regexp_replace(tablename, '.*_(\\d{4})_(\\d{2})$', '\\1-\\2-01')::date < date '${CUTOFF_DATE}'
ORDER BY tablename;
"

PARTITIONS=$(sudo -u postgres psql -d adproject -t -A -F '|' -c "${PARTITIONS_SQL}" 2>/dev/null || echo "")

if [[ -z "$PARTITIONS" ]]; then
  log "No rollup partitions eligible for archive."
  exit 0
fi

log "Found rollup partitions to archive:"
TOTAL_SIZE=0
while IFS='|' read -r tablename partition_month size_bytes; do
  log "  - ${tablename} (${partition_month})"
  TOTAL_SIZE=$((TOTAL_SIZE + size_bytes))
  if [[ "$DRY_RUN" == "true" ]]; then
    continue
  fi

  dump_file="$BACKUP_DIR/${tablename}.dump"
  if [[ ! -f "$dump_file" ]]; then
    log "  Exporting ${tablename}..."
    sudo -u postgres pg_dump -Fc -Z 9 -t "public.${tablename}" -f "$dump_file" adproject
  fi

  dest="${S3_BUCKET%/}/${S3_PREFIX}${tablename}.dump"
  aws_args=(/usr/local/bin/aws s3 cp "$dump_file" "$dest")
  if [[ -n "${S3_ENDPOINT:-}" ]]; then
    aws_args+=(--endpoint-url "$S3_ENDPOINT")
  fi

  log "  Uploading ${tablename} -> ${dest}"
  "${aws_args[@]}"

  log "  Dropping ${tablename} after successful upload"
  sudo -u postgres psql -d adproject -c "DROP TABLE IF EXISTS public.${tablename};" >/dev/null

  rm -f "$dump_file"
  log "  Archived ${tablename}"

done <<< "$PARTITIONS"

log "Archive run complete. Total archived size: $(numfmt --to=iec ${TOTAL_SIZE} 2>/dev/null || echo "${TOTAL_SIZE} bytes")"

# Refresh stats on parent tables
if [[ "$DRY_RUN" != "true" ]]; then
  sudo -u postgres psql -d adproject -c "VACUUM ANALYZE analytics_metrics_rollups;" >/dev/null 2>&1 || true
  sudo -u postgres psql -d adproject -c "VACUUM ANALYZE analytics_revenue_rollups;" >/dev/null 2>&1 || true
fi
