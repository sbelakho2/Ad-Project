#!/usr/bin/env bash
# Auto-create future partitions for time-series tables
# Run daily via cron to ensure partitions exist far enough ahead to keep health green
#
# Usage: ./scripts/ops/create_future_partitions.sh [days_forward] [days_back]

set -euo pipefail

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

DAILY_FORWARD=${1:-70}
DAILY_BACK=${2:-2}

log "Creating future partitions for time-series tables..."

PUBLIC_TABLES=(
  analytics_impressions
  analytics_revenue_events
)

EVENTS_TABLES=(
  ad_requests
  impressions
  clicks
  revenue
  bid_responses
  sdk_telemetry
)

# Create partitions from DAYS_BACK days ago through DAYS_FORWARD days ahead
start=-$DAILY_BACK
end=$DAILY_FORWARD

for ((i=start; i<=end; i++)); do
  # Calculate date (works on both GNU and BSD date)
  PARTITION_DATE=$(date -d "+${i} days" '+%Y-%m-%d' 2>/dev/null || date -v+${i}d '+%Y-%m-%d')
  NEXT_DATE=$(date -d "+$((i+1)) days" '+%Y-%m-%d' 2>/dev/null || date -v+$((i+1))d '+%Y-%m-%d')
  
  # Format for partition name (YYYY_MM_DD)
  PARTITION_SUFFIX=$(echo "$PARTITION_DATE" | tr '-' '_')
  
  for t in "${PUBLIC_TABLES[@]}"; do
    sudo -u postgres psql -d adproject -c "
      DO \$\$
      BEGIN
        IF EXISTS (
          SELECT 1 FROM pg_partitioned_table pt
          JOIN pg_class c ON pt.partrelid = c.oid
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'public' AND c.relname = '${t}'
        ) THEN
          IF NOT EXISTS (
            SELECT 1 FROM pg_tables 
            WHERE schemaname = 'public' 
            AND tablename = '${t}_${PARTITION_SUFFIX}'
          ) THEN
            EXECUTE format(
              'CREATE TABLE public.%I PARTITION OF public.%I FOR VALUES FROM (%L) TO (%L)',
              '${t}_${PARTITION_SUFFIX}',
              '${t}',
              '${PARTITION_DATE}',
              '${NEXT_DATE}'
            );
            RAISE NOTICE 'Created partition: ${t}_${PARTITION_SUFFIX}';
          END IF;
        END IF;
      END \$\$;
    " 2>/dev/null || log "Warning: Could not check/create ${t} partition for ${PARTITION_DATE}"
  done

  for t in "${EVENTS_TABLES[@]}"; do
    sudo -u postgres psql -d adproject -c "
      SELECT ops.create_daily_partition('${t}', 'events', '${PARTITION_DATE}'::date)
      WHERE EXISTS (
        SELECT 1
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'events' AND c.relname = '${t}' AND c.relkind = 'p'
      );
    " 2>/dev/null || log "Warning: Could not check/create events.${t} partition for ${PARTITION_DATE}"
  done
done

log "Future partitions ensured for ${DAILY_BACK} days back and ${DAILY_FORWARD} days forward"
