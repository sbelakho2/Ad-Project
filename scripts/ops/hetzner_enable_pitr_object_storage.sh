#!/usr/bin/env bash
set -euo pipefail

# Enable PITR-quality WAL + basebackup uploads to S3-compatible object storage on Hetzner DB server.
# Requires S3 credentials (AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY) and a bucket URL.
#
# Required env vars:
#   S3_BUCKET
#   AWS_ACCESS_KEY_ID
#   AWS_SECRET_ACCESS_KEY
# Optional env vars:
#   AWS_DEFAULT_REGION
#   S3_ENDPOINT        # S3-compatible endpoint (e.g. https://fsn1.your-objectstorage.com)
#   S3_WAL_PREFIX      # default: wal/
#   S3_BASE_PREFIX     # default: base/
#   WAL_DIR            # default: /mnt/HC_Volume_104191119/postgresql-backups/wal

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

require() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "[ERROR] Missing required env var: $name" >&2
    exit 2
  fi
}

require S3_BUCKET
require AWS_ACCESS_KEY_ID
require AWS_SECRET_ACCESS_KEY

S3_WAL_PREFIX=${S3_WAL_PREFIX:-wal/}
S3_BASE_PREFIX=${S3_BASE_PREFIX:-base/}
WAL_DIR=${WAL_DIR:-/mnt/HC_Volume_104191119/postgresql-backups/wal}

cat <<'BASH' | "$ROOT_DIR/scripts/ops/hetzner_ssh.sh" db -- "S3_BUCKET=${S3_BUCKET} AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID} AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY} AWS_DEFAULT_REGION=${AWS_DEFAULT_REGION:-} S3_ENDPOINT=${S3_ENDPOINT:-} S3_WAL_PREFIX=${S3_WAL_PREFIX} S3_BASE_PREFIX=${S3_BASE_PREFIX} WAL_DIR=${WAL_DIR} bash -s"
set -euo pipefail

# Install AWS CLI if needed (use a local venv to avoid system Python restrictions)
if ! command -v aws >/dev/null 2>&1; then
  apt-get update -y
  apt-get install -y python3-venv
  python3 -m venv /opt/adproject/ops/awscli-venv
  /opt/adproject/ops/awscli-venv/bin/pip install -U pip awscli
  ln -sf /opt/adproject/ops/awscli-venv/bin/aws /usr/local/bin/aws
  chmod 755 /usr/local/bin/aws
fi

install -d -m 0755 /opt/adproject/ops

cat <<'ENV' | tee /opt/adproject/ops/s3_archive.env >/dev/null
S3_BUCKET="${S3_BUCKET}"
AWS_ACCESS_KEY_ID="${AWS_ACCESS_KEY_ID}"
AWS_SECRET_ACCESS_KEY="${AWS_SECRET_ACCESS_KEY}"
AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-}"
S3_ENDPOINT="${S3_ENDPOINT:-}"
S3_WAL_PREFIX="${S3_WAL_PREFIX}"
S3_BASE_PREFIX="${S3_BASE_PREFIX}"
WAL_DIR="${WAL_DIR}"
ENV
chown postgres:postgres /opt/adproject/ops/s3_archive.env
chmod 0600 /opt/adproject/ops/s3_archive.env

cat <<'SH' | tee /opt/adproject/ops/pg_wal_archive_s3.sh >/dev/null
#!/usr/bin/env bash
set -euo pipefail

WAL_PATH="$1"
WAL_FILE="$2"

if [[ -f /opt/adproject/ops/s3_archive.env ]]; then
  set -a
  # shellcheck disable=SC1091
  source /opt/adproject/ops/s3_archive.env
  set +a
fi

WAL_DIR="${WAL_DIR:-/mnt/HC_Volume_104191119/postgresql-backups/wal}"
S3_WAL_PREFIX="${S3_WAL_PREFIX:-wal/}"

ARCHIVE_FILE="$WAL_DIR/${WAL_FILE}.gz"
mkdir -p "$WAL_DIR"

if [[ ! -f "$ARCHIVE_FILE" ]]; then
  tmp="$ARCHIVE_FILE.tmp"
  gzip -c "$WAL_PATH" > "$tmp"
  mv "$tmp" "$ARCHIVE_FILE"
fi

if [[ -z "${S3_BUCKET:-}" || -z "${AWS_ACCESS_KEY_ID:-}" || -z "${AWS_SECRET_ACCESS_KEY:-}" ]]; then
  echo "[ERROR] Missing S3 credentials/env; refusing to ack WAL archive." >&2
  exit 1
fi

if [[ -n "${AWS_DEFAULT_REGION:-}" ]]; then
  export AWS_DEFAULT_REGION
fi

aws_args=(/usr/local/bin/aws s3 cp "$ARCHIVE_FILE" "${S3_BUCKET%/}/${S3_WAL_PREFIX}${WAL_FILE}.gz")
if [[ -n "${S3_ENDPOINT:-}" ]]; then
  aws_args+=(--endpoint-url "$S3_ENDPOINT")
fi

"${aws_args[@]}"
SH
chmod 0755 /opt/adproject/ops/pg_wal_archive_s3.sh

cat <<'SH' | tee /opt/adproject/ops/pg_basebackup_upload_s3.sh >/dev/null
#!/usr/bin/env bash
set -euo pipefail

if [[ -f /opt/adproject/ops/s3_archive.env ]]; then
  set -a
  # shellcheck disable=SC1091
  source /opt/adproject/ops/s3_archive.env
  set +a
fi

BASE_DIR="/mnt/HC_Volume_104191119/postgresql-backups/base"
S3_BASE_PREFIX="${S3_BASE_PREFIX:-base/}"

latest=$(ls -1t "$BASE_DIR"/base_*.tar.gz 2>/dev/null | head -n 1 || true)
if [[ -z "$latest" ]]; then
  echo "[WARN] No base backup found in $BASE_DIR" >&2
  exit 0
fi

if [[ -z "${S3_BUCKET:-}" || -z "${AWS_ACCESS_KEY_ID:-}" || -z "${AWS_SECRET_ACCESS_KEY:-}" ]]; then
  echo "[ERROR] Missing S3 credentials/env; refusing to upload base backup." >&2
  exit 1
fi

if [[ -n "${AWS_DEFAULT_REGION:-}" ]]; then
  export AWS_DEFAULT_REGION
fi

fname=$(basename "$latest")
aws_args=(/usr/local/bin/aws s3 cp "$latest" "${S3_BUCKET%/}/${S3_BASE_PREFIX}${fname}")
if [[ -n "${S3_ENDPOINT:-}" ]]; then
  aws_args+=(--endpoint-url "$S3_ENDPOINT")
fi

"${aws_args[@]}"
SH
chmod 0755 /opt/adproject/ops/pg_basebackup_upload_s3.sh

cat <<'CONF' | tee /etc/postgresql/16/main/conf.d/99-adproject-archive-s3.conf >/dev/null
# AdProject: archive WAL to local disk + S3-compatible object storage
archive_command = '/opt/adproject/ops/pg_wal_archive_s3.sh %p %f'
CONF

systemctl reload postgresql

cat <<'CRON' | tee /etc/cron.d/adproject-pitr-s3 >/dev/null
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Upload the most recent basebackup to object storage (after nightly base backup)
30 2 * * * root /opt/adproject/ops/pg_basebackup_upload_s3.sh >>/var/log/adproject-pitr-s3.log 2>&1
CRON
chmod 0644 /etc/cron.d/adproject-pitr-s3

echo "OK: S3 WAL/base backup archiving configured."
BASH
