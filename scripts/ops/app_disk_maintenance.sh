#!/usr/bin/env bash
set -euo pipefail

KEEP_BACKUPS=${KEEP_BACKUPS:-2}
KEEP_DAYS_DOCKER=${KEEP_DAYS_DOCKER:-7}
DEPLOY_PATH=${DEPLOY_PATH:-/opt/adproject}

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

log "Pruning old deploy backups (keep last ${KEEP_BACKUPS})"
if compgen -G "$DEPLOY_PATH/console_backup_*" > /dev/null; then
  ls -1dt "$DEPLOY_PATH"/console_backup_* | tail -n +$((KEEP_BACKUPS+1)) | xargs -r rm -rf
fi
if compgen -G "$DEPLOY_PATH/website_backup_*" > /dev/null; then
  ls -1dt "$DEPLOY_PATH"/website_backup_* | tail -n +$((KEEP_BACKUPS+1)) | xargs -r rm -rf
fi
if compgen -G "$DEPLOY_PATH/control-plane.backup.*" > /dev/null; then
  ls -1dt "$DEPLOY_PATH"/control-plane.backup.* | tail -n +$((KEEP_BACKUPS+1)) | xargs -r rm -rf
fi

KEEP_HOURS_DOCKER=$((KEEP_DAYS_DOCKER * 24))
log "Pruning Docker images/build cache older than ${KEEP_DAYS_DOCKER} days (${KEEP_HOURS_DOCKER}h)"
docker system prune -af --filter "until=${KEEP_HOURS_DOCKER}h" || true

log "App disk maintenance complete"
