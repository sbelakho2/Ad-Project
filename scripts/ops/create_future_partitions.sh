#!/usr/bin/env bash
# Auto-create future partitions for analytics tables
# Run daily via cron to ensure partitions exist 7 days ahead
#
# Usage: ./scripts/ops/create_future_partitions.sh

set -euo pipefail

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

log "Creating future partitions for analytics tables..."

# Create partitions for the next 7 days
for i in {0..7}; do
  # Calculate date (works on both GNU and BSD date)
  PARTITION_DATE=$(date -d "+${i} days" '+%Y-%m-%d' 2>/dev/null || date -v+${i}d '+%Y-%m-%d')
  NEXT_DATE=$(date -d "+$((i+1)) days" '+%Y-%m-%d' 2>/dev/null || date -v+$((i+1))d '+%Y-%m-%d')
  
  # Format for partition name (YYYY_MM_DD)
  PARTITION_SUFFIX=$(echo "$PARTITION_DATE" | tr '-' '_')
  
  # Create impressions partition if it doesn't exist
  sudo -u postgres psql -d adproject -c "
    DO \$\$
    BEGIN
      IF NOT EXISTS (
        SELECT 1 FROM pg_tables 
        WHERE schemaname = 'public' 
        AND tablename = 'analytics_impressions_${PARTITION_SUFFIX}'
      ) THEN
        CREATE TABLE public.analytics_impressions_${PARTITION_SUFFIX} 
        PARTITION OF public.analytics_impressions
        FOR VALUES FROM ('${PARTITION_DATE}') TO ('${NEXT_DATE}');
        RAISE NOTICE 'Created partition: analytics_impressions_${PARTITION_SUFFIX}';
      END IF;
    END \$\$;
  " 2>/dev/null || log "Warning: Could not check/create impressions partition for ${PARTITION_DATE}"
  
  # Create revenue_events partition if it doesn't exist (only if table is partitioned)
  sudo -u postgres psql -d adproject -c "
    DO \$\$
    BEGIN
      -- Check if analytics_revenue_events is partitioned
      IF EXISTS (
        SELECT 1 FROM pg_partitioned_table pt
        JOIN pg_class c ON pt.partrelid = c.oid
        WHERE c.relname = 'analytics_revenue_events'
      ) THEN
        IF NOT EXISTS (
          SELECT 1 FROM pg_tables 
          WHERE schemaname = 'public' 
          AND tablename = 'analytics_revenue_events_${PARTITION_SUFFIX}'
        ) THEN
          CREATE TABLE public.analytics_revenue_events_${PARTITION_SUFFIX} 
          PARTITION OF public.analytics_revenue_events
          FOR VALUES FROM ('${PARTITION_DATE}') TO ('${NEXT_DATE}');
          RAISE NOTICE 'Created partition: analytics_revenue_events_${PARTITION_SUFFIX}';
        END IF;
      END IF;
    END \$\$;
  " 2>/dev/null || log "Warning: Could not check/create revenue_events partition for ${PARTITION_DATE}"
done

log "Future partitions ensured for next 7 days"
