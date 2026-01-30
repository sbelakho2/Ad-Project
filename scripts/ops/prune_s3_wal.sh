#!/usr/bin/env bash
set -euo pipefail
# Prune S3 WAL archives older than retention period
# Default: 7 days (matches business-safe local retention)

RETENTION_DAYS="${1:-7}"
LOG="/var/log/adproject-s3-wal-prune.log"

if [[ -f /opt/adproject/ops/s3_archive.env ]]; then
  set -a
  source /opt/adproject/ops/s3_archive.env
  set +a
fi

export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_DEFAULT_REGION

CUTOFF=$(date -d "-${RETENTION_DAYS} days" +%Y-%m-%d)

echo "$(date +%F_%T) - Pruning S3 WAL older than ${CUTOFF} (retention=${RETENTION_DAYS}d)" >> "$LOG"

count=0
# List and delete files older than cutoff
/usr/local/bin/aws s3 ls "${S3_BUCKET}/${S3_WAL_PREFIX}" --endpoint-url "$S3_ENDPOINT" 2>/dev/null | while read -r line; do
  file_date=$(echo "$line" | awk '{print $1}')
  file_name=$(echo "$line" | awk '{print $4}')
  if [[ "$file_date" < "$CUTOFF" && -n "$file_name" ]]; then
    /usr/local/bin/aws s3 rm "${S3_BUCKET}/${S3_WAL_PREFIX}${file_name}" --endpoint-url "$S3_ENDPOINT" 2>/dev/null
    ((count++)) || true
  fi
done

echo "$(date +%F_%T) - S3 WAL prune complete, deleted ${count:-0} files" >> "$LOG"
