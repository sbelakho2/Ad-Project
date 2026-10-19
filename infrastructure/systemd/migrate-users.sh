#!/bin/sh
# Apex per-service systemd users and state-layout migration.
#
# Creates the shared group "apex" and one unprivileged system user per
# long-running service (no login shell, no home), then normalizes the
# documented state layout under APEX_STATE_ROOT (default /var/lib/apex):
# setgid directories owned by-group "apex" at mode 2770, data files at mode
# 0660, and each writing service's own directory owned by that service. The
# vault/credential files stay 0640.
#
# It is root-only and idempotent: running it twice is a no-op that reports the
# same layout. No service is restarted here; install the units and run
# "systemctl daemon-reload" afterwards (see infrastructure/README.md).
#
#   sudo sh infrastructure/systemd/migrate-users.sh
#
# Environment overrides:
#   APEX_STATE_ROOT   state root (default /var/lib/apex)
#   APEX_ACCESS_LOG   Caddy access log (default /var/log/caddy/apex-access.json)

set -eu

STATE_ROOT=${APEX_STATE_ROOT:-/var/lib/apex}
STATE_DIR="$STATE_ROOT/.apex"
ACCESS_LOG=${APEX_ACCESS_LOG:-/var/log/caddy/apex-access.json}
GROUP=apex
SERVICES="apex-api apex-control apex-runner apex-ingest apex-watch apex-sales apex-sales-web apex-sync"

# The repository's destructive-script contract: a dry run reports the plan and
# changes nothing (and does not need root).
if [ -n "${DOGFOOD_DRY_RUN:-}" ] && [ "${DOGFOOD_DRY_RUN}" != "0" ]; then
    echo "migrate-users.sh: dry run; would create group $GROUP, users:$SERVICES, and normalize $STATE_ROOT"
    exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "migrate-users.sh: must be run as root (try: sudo sh $0)" >&2
    exit 1
fi

have() { command -v "$1" >/dev/null 2>&1; }

# getent when present, the flat files otherwise.
group_exists() {
    if have getent; then getent group "$1" >/dev/null 2>&1; else grep -q "^$1:" /etc/group; fi
}

user_exists() {
    if have getent; then getent passwd "$1" >/dev/null 2>&1; else grep -q "^$1:" /etc/passwd; fi
}

log() { printf '  %s\n' "$*"; }

summary_group=""
summary_users=""

# --- no-login shell ---------------------------------------------------------
NOLOGIN=
for candidate in /usr/sbin/nologin /sbin/nologin /usr/bin/false /bin/false; do
    if [ -x "$candidate" ]; then
        NOLOGIN=$candidate
        break
    fi
done
if [ -z "$NOLOGIN" ]; then
    echo "migrate-users.sh: no no-login shell found (nologin/false)" >&2
    exit 1
fi

# --- group "apex" -----------------------------------------------------------
if group_exists "$GROUP"; then
    summary_group="group $GROUP already present"
elif have groupadd; then
    groupadd --system "$GROUP"
    summary_group="created group $GROUP (groupadd --system)"
elif have addgroup; then
    addgroup --system "$GROUP"
    summary_group="created group $GROUP (addgroup --system)"
else
    echo "migrate-users.sh: neither groupadd nor addgroup is available" >&2
    exit 1
fi

# --- service users ----------------------------------------------------------
for user in $SERVICES; do
    if getent passwd "$user" >/dev/null 2>&1; then
        summary_users="$summary_users $user(existing)"
        continue
    fi
    if have useradd; then
        useradd --system --gid "$GROUP" --home-dir /nonexistent \
            --no-create-home --shell "$NOLOGIN" "$user"
    elif have adduser; then
        adduser --system --ingroup "$GROUP" --no-create-home \
            --home /nonexistent --shell "$NOLOGIN" "$user"
    else
        echo "migrate-users.sh: neither useradd nor adduser is available" >&2
        exit 1
    fi
    summary_users="$summary_users $user(created)"
done

# --- state layout -----------------------------------------------------------
# install -d is idempotent and applies owner/group/mode to an existing path.
ensure_dir() {
    path=$1
    owner=$2
    mode=$3
    install -d -o "$owner" -g "$GROUP" -m "$mode" "$path"
    chown -R "$owner:$GROUP" "$path"
}

summary_dirs=""
install -d -o root -g "$GROUP" -m 2770 "$STATE_ROOT"
install -d -o root -g "$GROUP" -m 2770 "$STATE_DIR"
summary_dirs="$STATE_ROOT $STATE_DIR"

# Each writing service owns its own directory; shared roots stay root:apex.
ensure_dir "$STATE_DIR/analytics-spool" apex-ingest 2770   # ingest spool + cursor
ensure_dir "$STATE_DIR/watch"           apex-watch  2770   # subscriber root
ensure_dir "$STATE_DIR/sales"           apex-sales  2770   # store, events, outbox
ensure_dir "$STATE_DIR/ledgers"         apex-runner 2770   # publisher ledgers
ensure_dir "$STATE_DIR/snapshots"       apex-runner 2770   # account snapshots
ensure_dir "$STATE_DIR/proposals"       apex-runner 2770   # proposal review store
ensure_dir "$STATE_DIR/priors"          apex-runner 2770   # bucket models
ensure_dir "$STATE_DIR/runner"          apex-runner 2770   # runner data dir
ensure_dir "$STATE_DIR/control"         apex-control 2770  # journal/policies/interventions
ensure_dir "$STATE_DIR/control/jobs"    apex-control 2770  # operator jobs
ensure_dir "$STATE_DIR/billing"         apex-runner 2770   # invoice/collection register
ensure_dir "$STATE_DIR/imports"         apex-sync   2770   # synced sales imports
ensure_dir "$STATE_DIR/credentials"     apex-control 2770  # vault / plan state
ensure_dir "$STATE_DIR/analytics"       root        2770   # per-service spools
ensure_dir "$STATE_DIR/notify"          root        2770   # delivery state (shared)

# Normalize the whole tree: group apex, setgid 2770 directories, 0660 files.
dir_count=$(find "$STATE_ROOT" -type d | wc -l | tr -d ' ')
file_count=$(find "$STATE_ROOT" -type f | wc -l | tr -d ' ')
find "$STATE_ROOT" -type d -exec chgrp "$GROUP" {} +
find "$STATE_ROOT" -type d -exec chmod 2770 {} +
find "$STATE_ROOT" -type f -exec chgrp "$GROUP" {} +
find "$STATE_ROOT" -type f -exec chmod 0660 {} +

# Vault credentials are secrets: keep them group-readable but not group-writable.
if [ -d "$STATE_DIR/credentials" ]; then
    find "$STATE_DIR/credentials" -type f -exec chmod 0640 {} +
fi

# The root-level lead and sign-in files are written by apex-api.
for name in leads.jsonl signin.jsonl; do
    file="$STATE_DIR/$name"
    if [ -e "$file" ]; then
        chown "apex-api:$GROUP" "$file"
        chmod 0660 "$file"
    fi
done
# The lead-id salt derives from <leads file>.salt; it is a secret, so keep it
# at 0640 rather than the 0660 data-file mode.
file="$STATE_DIR/leads.jsonl.salt"
if [ -e "$file" ]; then
    chown "apex-api:$GROUP" "$file"
    chmod 0640 "$file"
fi

summary_dirs="$summary_dirs (normalized $dir_count dirs, $file_count files)"

# --- access log read access for apex-ingest ---------------------------------
# The units set SupplementaryGroups=apex, which replaces the /etc/group
# supplementary groups, so a "caddy" group membership would not survive.
# Grant the ingest user read access with an ACL instead (and inherit it onto
# rotated files through the default ACL), when setfacl is available.
summary_acl="skipped (setfacl unavailable)"
if have setfacl; then
    logdir=$(dirname "$ACCESS_LOG")
    if [ -d "$logdir" ] && getent passwd apex-ingest >/dev/null 2>&1; then
        setfacl -m u:apex-ingest:x "$logdir"
        setfacl -d -m u:apex-ingest:r "$logdir"
        if [ -e "$ACCESS_LOG" ]; then
            setfacl -m u:apex-ingest:r "$ACCESS_LOG"
        fi
        summary_acl="apex-ingest granted read on $ACCESS_LOG (ACL)"
    else
        summary_acl="skipped ($logdir or apex-ingest missing)"
    fi
fi

# --- summary ----------------------------------------------------------------
echo "migrate-users.sh: migration complete"
log "state root: $STATE_ROOT"
log "group: $summary_group"
log "users:$summary_users"
log "layout: $summary_dirs"
log "access log: $summary_acl"
log "next: install the units and run 'systemctl daemon-reload'"
