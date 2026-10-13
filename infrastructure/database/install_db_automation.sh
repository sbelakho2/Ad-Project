#!/usr/bin/env bash
# Install ApexMediation DB automation on a PostgreSQL DB server.
#
# This script is intended to be run ON the DB server as root.
# It installs:
# - /usr/local/bin/* scripts (backups, WAL cleanup, health checks)
# - /etc/cron.d/apex-postgres
# - backup directories under /var/backups/postgresql

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC_DIR="$REPO_ROOT/infrastructure/database"

install -d -m 0755 /usr/local/bin
install -d -m 0755 /etc/cron.d
install -d -m 0755 /var/backups/postgresql/daily /var/backups/postgresql/weekly /var/backups/postgresql/wal /var/backups/postgresql/base
install -d -m 0755 /var/log/postgresql

# Ensure backup dirs are owned by postgres
chown -R postgres:postgres /var/backups/postgresql
chown -R postgres:postgres /var/log/postgresql || true

# Scripts
install -m 0755 "$SRC_DIR/pg_backup_apex.sh" /usr/local/bin/pg_backup_apex.sh
install -m 0755 "$SRC_DIR/bin/pg_wal_cleanup.sh" /usr/local/bin/pg_wal_cleanup.sh
install -m 0755 "$SRC_DIR/bin/pg_basebackup_apex.sh" /usr/local/bin/pg_basebackup_apex.sh
install -m 0755 "$SRC_DIR/bin/pg_autonomous_health.sh" /usr/local/bin/pg_autonomous_health.sh
install -m 0755 "$SRC_DIR/bin/check-disk-space.sh" /usr/local/bin/check-disk-space.sh

# Cron
install -m 0644 "$SRC_DIR/cron/apex-postgres" /etc/cron.d/apex-postgres

echo "[ok] Installed DB automation scripts and cron entries."

echo "[next] Verify cron is running: systemctl status cron || systemctl status crond"
echo "[next] Verify logs: /var/log/postgresql/backup.log, /var/log/postgresql/wal-cleanup.log, /var/log/postgresql/health_check.log"
