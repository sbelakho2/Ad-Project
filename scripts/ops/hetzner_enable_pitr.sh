#!/usr/bin/env bash
set -euo pipefail

# Idempotently enforce PITR-quality backups on Hetzner DB server:
# - Keep ~72h worth of archived WAL (3-day PITR window, weekend-safe)
# - Daily physical base backup
# - Reduce WAL growth by making staging tables UNLOGGED + enabling wal_compression
#
# Requirements:
# - SSH key at ~/.ssh/hetzner-db-mac or set APEX_HETZNER_SSH_KEY
# - DB host is reached via app server jump (handled by hetzner_ssh.sh)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WAL_ARCHIVE_DIR=${WAL_ARCHIVE_DIR:-}

cat <<'BASH' | "$ROOT_DIR/scripts/ops/hetzner_ssh.sh" db -- "WAL_ARCHIVE_DIR=${WAL_ARCHIVE_DIR} bash -s"
set -euo pipefail

DEFAULT_WAL_DIR="/var/backups/postgresql/wal"
WAL_DIR="${WAL_ARCHIVE_DIR:-$DEFAULT_WAL_DIR}"

if [[ "$WAL_DIR" != "$DEFAULT_WAL_DIR" ]]; then
  echo "== 0) Relocating WAL archive dir to $WAL_DIR =="
  sudo install -d -m 0755 "$WAL_DIR"
  if [[ -e "$DEFAULT_WAL_DIR" && ! -L "$DEFAULT_WAL_DIR" ]]; then
    sudo rsync -a "$DEFAULT_WAL_DIR"/ "$WAL_DIR"/ || true
    sudo rm -rf "$DEFAULT_WAL_DIR"
  fi
  if [[ ! -L "$DEFAULT_WAL_DIR" ]]; then
    sudo ln -s "$WAL_DIR" "$DEFAULT_WAL_DIR"
  fi
fi

echo '== 1) Ensure hot staging tables are UNLOGGED (truncate first) =='
sudo -u postgres psql -d adproject -v ON_ERROR_STOP=1 -P pager=off <<'SQL'
DO $$
DECLARE
  tbl text;
BEGIN
  FOREACH tbl IN ARRAY ARRAY[
    'analytics_impressions_stage',
    'analytics_clicks_stage',
    'analytics_revenue_events_stage',
    'analytics_bid_landscape_stage',
    'analytics_performance_metrics_stage',
    'analytics_sdk_telemetry_stage',
    'analytics_quality_alerts_stage',
    'analytics_creative_scans_stage',
    'analytics_creative_compliance_stage',
    'transparency_auctions_stage',
    'transparency_auction_candidates_stage'
  ]
  LOOP
    IF EXISTS (
      select 1
      from pg_class c
      join pg_namespace n on n.oid=c.relnamespace
      where n.nspname='public' and c.relkind='r' and c.relname=tbl
    ) THEN
      EXECUTE format('TRUNCATE TABLE public.%I', tbl);
      EXECUTE format('ALTER TABLE public.%I SET UNLOGGED', tbl);
    END IF;
  END LOOP;
END $$;
SQL

echo '== 2) WAL tuning (compression + gentler checkpoints) =='
CONF_DIR="/etc/postgresql/16/main/conf.d"
sudo mkdir -p "$CONF_DIR"

cat <<'CONF' | sudo tee "$CONF_DIR/99-adproject-wal.conf" >/dev/null
# AdProject: reduce WAL volume while preserving correctness
wal_compression = lz4
CONF

cat <<'CONF' | sudo tee "$CONF_DIR/zzzz-adproject-wal-tuning.conf" >/dev/null
# AdProject: late override (loads after apex_performance.conf)
max_wal_size = '8GB'
checkpoint_timeout = '30min'
checkpoint_completion_target = 0.9
# Reduce archived-WAL disk usage on low-traffic systems by avoiding frequent
# forced WAL segment switches. This increases worst-case RPO (for catastrophic
# loss of the primary) up to the archive_timeout window.
archive_timeout = '1h'
CONF

# Allow local replication connections for pg_basebackup
HBA="/etc/postgresql/16/main/pg_hba.conf"
LINE='local   replication   postgres                                peer'
if ! grep -Eq '^local\s+replication\s+postgres\s+peer\s*$' "$HBA"; then
  tmp=$(mktemp)
  printf "%s\n" "$LINE" >"$tmp"
  cat "$HBA" >>"$tmp"
  sudo mv "$tmp" "$HBA"
  sudo chown postgres:postgres "$HBA"
  sudo chmod 640 "$HBA"
fi

sudo systemctl reload postgresql

echo '== 3) PITR automation (daily base backup + 72h WAL retention) =='
sudo install -d -m 0755 /opt/adproject/ops

KEEP_HOURS="72"
KEEP_MINUTES="$((KEEP_HOURS*60))"

cat <<'SH' | sudo tee /opt/adproject/ops/pitr_basebackup.sh >/dev/null
#!/usr/bin/env bash
set -euo pipefail

BASE_DIR="/mnt/HC_Volume_104191119/postgresql-backups/base"
mkdir -p "$BASE_DIR"
STAMP=$(date -u +%F)
OUT="$BASE_DIR/base_${STAMP}.tar.gz"
TMP="$BASE_DIR/.tmp_base_${STAMP}"

LOCKFILE="$BASE_DIR/.basebackup.lock"
exec 9>"$LOCKFILE"
if ! flock -n 9; then
  exit 0
fi

rm -rf "$TMP"
mkdir -p "$TMP"
chown postgres:postgres "$TMP"

sudo -u postgres pg_basebackup -D "$TMP" -F p -X stream -c fast -P

tar -C "$TMP" -czf "$OUT" .
rm -rf "$TMP"

# keep last 3 base backups
ls -1t "$BASE_DIR"/base_*.tar.gz 2>/dev/null | tail -n +4 | xargs -r rm -f
SH

sudo chmod +x /opt/adproject/ops/pitr_basebackup.sh

cat <<SH | sudo tee /opt/adproject/ops/prune_wal_archives.sh >/dev/null
#!/usr/bin/env bash
set -euo pipefail
WAL_DIR="$WAL_DIR"
KEEP_HOURS="72"

[[ -d "$WAL_DIR" ]] || exit 0

find "$WAL_DIR" -type f -name '0000*' -mmin "+$((KEEP_HOURS*60))" -print0 | xargs -0 -r rm -f
SH

sudo chmod +x /opt/adproject/ops/prune_wal_archives.sh

# IMPORTANT: legacy automation may also prune WAL archives.
# Ensure any existing retention jobs do not override the >=24h PITR window.
if [[ -f /etc/cron.d/apex-postgres-backup-retention ]]; then
  # Replace the WAL retention window (commonly 12h = 720min) with KEEP_MINUTES.
  sudo sed -i -E "s@(find /var/backups/postgresql/wal[^\n]*-mmin \+)[0-9]+@\1${KEEP_MINUTES}@g" /etc/cron.d/apex-postgres-backup-retention || true
fi

cat <<'CRON' | sudo tee /etc/cron.d/adproject-pitr >/dev/null
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Daily physical basebackup (PITR)
15 2 * * * root /opt/adproject/ops/pitr_basebackup.sh >>/var/log/adproject-pitr.log 2>&1
CRON

cat <<'CRON' | sudo tee /etc/cron.d/adproject-wal-retention >/dev/null
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Keep ~72h worth of archived WAL for PITR window (weekend-safe)
7 * * * * root /opt/adproject/ops/prune_wal_archives.sh >>/var/log/adproject-wal-retention.log 2>&1
CRON

sudo chmod 0644 /etc/cron.d/adproject-pitr /etc/cron.d/adproject-wal-retention

echo '== 4) Snapshot =='
df -hT /
du -sh "$WAL_DIR" /mnt/HC_Volume_104191119/postgresql-backups/base /var/lib/postgresql/16/main 2>/dev/null || true
sudo -u postgres psql -d adproject -P pager=off -At <<'SQL'
select 'db_size='||pg_size_pretty(pg_database_size('adproject'));
select 'archived_count='||archived_count||' failed_count='||failed_count from pg_stat_archiver;
select name||'='||setting||' source='||coalesce(sourcefile,'') from pg_settings where name in ('wal_compression','max_wal_size','checkpoint_timeout');
SQL

echo 'OK: PITR automation ensured (UNLOGGED staging, wal_compression, daily basebackup, 72h WAL retention)'
BASH
