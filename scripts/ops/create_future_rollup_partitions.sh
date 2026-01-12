#!/usr/bin/env bash
# Auto-create monthly partitions for analytics rollup tables.
# Keeps rollups cheap to drop for multi-year retention.
#
# Usage:
#   ./scripts/ops/create_future_rollup_partitions.sh [months_back] [months_forward]
#
# Defaults:
#   months_back=2 (ensure partitions exist for backfill runs)
#   months_forward=24

set -euo pipefail

MONTHS_BACK=${1:-2}
MONTHS_FORWARD=${2:-24}

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

# Linux (GNU date) preferred; fallback for BSD date (local dev).
add_months_ymd() {
  local ymd="$1"
  local delta="$2"
  if date -d "$ymd" '+%Y-%m-%d' >/dev/null 2>&1; then
    date -d "$ymd ${delta} month" '+%Y-%m-%d'
  else
    # macOS: expects -v+1m style
    if [[ "$delta" -ge 0 ]]; then
      date -j -f '%Y-%m-%d' "$ymd" -v+"${delta}"m '+%Y-%m-%d'
    else
      local abs=$(( -delta ))
      date -j -f '%Y-%m-%d' "$ymd" -v-"${abs}"m '+%Y-%m-%d'
    fi
  fi
}

first_of_month() {
  local ymd="$1"
  echo "${ymd:0:7}-01"
}

table_list=(
  analytics_impression_rollups
  analytics_click_rollups
  analytics_revenue_rollups
  analytics_metrics_rollups
)

log "Creating monthly partitions for rollup tables (back=${MONTHS_BACK} forward=${MONTHS_FORWARD})"

TODAY=$(date -u '+%Y-%m-%d')
START_MONTH=$(add_months_ymd "$(first_of_month "$TODAY")" "-$MONTHS_BACK")
END_MONTH=$(add_months_ymd "$(first_of_month "$TODAY")" "$MONTHS_FORWARD")

cur="$START_MONTH"
while [[ "$cur" < "$END_MONTH" || "$cur" == "$END_MONTH" ]]; do
  next=$(add_months_ymd "$cur" 1)
  suffix=$(echo "$cur" | cut -d'-' -f1-2 | tr '-' '_')

  for t in "${table_list[@]}"; do
    sudo -u postgres psql -d adproject -c "
      DO \$\$
      BEGIN
        IF EXISTS (
          SELECT 1 FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname='public' AND c.relname = '${t}'
        ) THEN
          IF NOT EXISTS (
            SELECT 1 FROM pg_tables
            WHERE schemaname='public' AND tablename='${t}_${suffix}'
          ) THEN
            EXECUTE format(
              'CREATE TABLE public.%I PARTITION OF public.%I FOR VALUES FROM (%L) TO (%L)',
              '${t}_${suffix}',
              '${t}',
              '${cur}',
              '${next}'
            );
            RAISE NOTICE 'Created rollup partition: %', '${t}_${suffix}';
          END IF;
        END IF;
      END \$\$;
    " 2>/dev/null || log "Warning: Could not check/create ${t}_${suffix}"
  done

  cur="$next"
done

log "Rollup partitions ensured."
