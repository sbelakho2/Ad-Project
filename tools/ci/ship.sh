#!/bin/sh
# ship.sh: build the selected services, copy the release binaries to the
# deploy host with rsync, back the previous binaries up on the remote side,
# then restart the systemd units. No secret material ever reaches argv: the
# ssh transport uses your ssh config and agent by reference. See
# tools/ci/ship.env.sample and docs/Internal/CI/LOCAL_CI.md.
# DOGFOOD_DRY_RUN=1 equals --dry-run.
set -eu

ALL_SERVICES="apex-api apex-control apex-sales-web apex-watch apex-runner apex-ingest"

usage() {
  cat <<'EOF'
usage: sh tools/ci/ship.sh [flags]

Build the selected services in release mode, rsync their binaries to
APEX_SHIP_HOST:APEX_SHIP_BIN_DIR, back the old binaries up remotely, then
systemctl restart the mapped units.

flags:
  --service <name>   ship one service (repeatable; default: all)
                     known: apex-api apex-control apex-sales-web apex-watch
                            apex-runner apex-ingest
  --host <host>      override APEX_SHIP_HOST (ssh alias or user@host)
  --dry-run          print the commands; execute nothing
  --no-build         skip cargo build; ship the existing artifacts
  --rollback         restore the newest remote backup and restart the units

environment:
  APEX_SHIP_HOST          deploy host (required unless --host)
  APEX_SHIP_BIN_DIR       remote binary dir (default: /usr/local/bin)
  APEX_SHIP_UNITS         units to restart; overrides the per-service map
  APEX_SHIP_TARGET        cargo cross target (for example
                          x86_64-unknown-linux-gnu); selects the artifact dir
                          rust/target/<target>/release
  APEX_SHIP_ARTIFACT_DIR  local artifact dir override
  APEX_SHIP_ENV           env file (default: tools/ci/ship.env when present)
  DOGFOOD_DRY_RUN=1       same as --dry-run
EOF
}

die() {
  printf '%s\n' "ship: $*" >&2
  exit 1
}

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)

DRY=0
case "${DOGFOOD_DRY_RUN:-0}" in
  1) DRY=1 ;;
  *) DRY=0 ;;
esac

SERVICES=""
HOST_OVERRIDE=""
ROLLBACK=0
NO_BUILD=0

while [ $# -gt 0 ]; do
  case "$1" in
    --service)
      [ $# -ge 2 ] || die "--service needs a service name"
      SERVICES="$SERVICES $2"
      shift 2
      ;;
    --host)
      [ $# -ge 2 ] || die "--host needs a host"
      HOST_OVERRIDE="$2"
      shift 2
      ;;
    --dry-run)
      DRY=1
      shift
      ;;
    --no-build)
      NO_BUILD=1
      shift
      ;;
    --rollback)
      ROLLBACK=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown flag: '$1' (see --help)"
      ;;
  esac
done

ENV_FILE="${APEX_SHIP_ENV:-$ROOT/tools/ci/ship.env}"
if [ -f "$ENV_FILE" ]; then
  # shellcheck source=/dev/null
  . "$ENV_FILE"
fi

HOST="${HOST_OVERRIDE:-${APEX_SHIP_HOST:-}}"
[ -n "$HOST" ] || die "no deploy host: set APEX_SHIP_HOST or pass --host (see tools/ci/ship.env.sample)"
BIN_DIR="${APEX_SHIP_BIN_DIR:-/usr/local/bin}"
TARGET="${APEX_SHIP_TARGET:-}"

if [ -z "$SERVICES" ]; then
  SERVICES="$ALL_SERVICES"
fi
UNIQUE=""
for svc in $SERVICES; do
  case " $ALL_SERVICES " in
    *" $svc "*) : ;;
    *) die "unknown service '$svc'; known services: $ALL_SERVICES" ;;
  esac
  case " $UNIQUE " in
    *" $svc "*) : ;;
    *) UNIQUE="$UNIQUE $svc" ;;
  esac
done
SERVICES="${UNIQUE# }"

if [ -n "${APEX_SHIP_ARTIFACT_DIR:-}" ]; then
  ART="$APEX_SHIP_ARTIFACT_DIR"
elif [ -n "$TARGET" ]; then
  ART="$ROOT/rust/target/$TARGET/release"
else
  ART="$ROOT/rust/target/release"
fi

pkg_for_service() {
  case "$1" in
    apex-runner) printf '%s' "apex-autopilot" ;;
    apex-ingest) printf '%s' "apex-analytics" ;;
    *) printf '%s' "$1" ;;
  esac
}

units_for_service() {
  case "$1" in
    apex-api) printf '%s\n' "apex-api.service" ;;
    apex-control) printf '%s\n' "apex-control.service" ;;
    apex-sales-web) printf '%s\n' "apex-sales-web.service" ;;
    apex-watch) printf '%s\n' "apex-watch.service" ;;
    apex-ingest) printf '%s\n' "apex-ingest.service" ;;
    apex-runner)
      printf '%s\n' "apex-runner-audit.service" "apex-runner-cycle.service" \
        "apex-runner-daily.service" "apex-runner-monthly.service"
      ;;
  esac
}

UNITS="${APEX_SHIP_UNITS:-}"
if [ -z "$UNITS" ]; then
  for svc in $SERVICES; do
    UNITS="$UNITS $(units_for_service "$svc")"
  done
  UNITS="${UNITS# }"
fi

quote_arg() {
  case "$1" in
    '')
      printf "''"
      ;;
    *[!A-Za-z0-9_/:=\.,@%+-]*)
      printf "'"
      printf '%s' "$1" | sed "s/'/'\\\\''/g"
      printf "'"
      ;;
    *)
      printf '%s' "$1"
      ;;
  esac
}

print_cmd() {
  line=""
  for a in "$@"; do
    line="$line $(quote_arg "$a")"
  done
  printf '%s\n' "${line# }"
}

build_services() {
  set -- cargo build --release
  if [ -n "$TARGET" ]; then
    set -- "$@" --target "$TARGET"
  fi
  for svc in $SERVICES; do
    set -- "$@" -p "$(pkg_for_service "$svc")"
  done
  if [ "$DRY" = 1 ]; then
    printf '%s\n' "# in $ROOT/rust"
    print_cmd "$@"
    return 0
  fi
  ( cd "$ROOT/rust" && "$@" )
}

check_binaries() {
  missing=""
  for svc in $SERVICES; do
    if [ ! -x "$ART/$svc" ]; then
      missing="$missing $svc"
    fi
  done
  if [ -n "$missing" ]; then
    die "missing or not executable in $ART:$missing; build first or set APEX_SHIP_TARGET / APEX_SHIP_ARTIFACT_DIR"
  fi
}

probe_host() {
  if [ "$DRY" = 1 ]; then
    print_cmd ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" true
    return 0
  fi
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" true
}

remote() {
  if [ "$DRY" = 1 ]; then
    print_cmd ssh "$HOST" "$1"
    return 0
  fi
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" "$1"
}

probe_units() {
  for unit in $UNITS; do
    remote "systemctl cat $(quote_arg "$unit") >/dev/null 2>&1" ||
      die "unit '$unit' is not installed on $HOST (systemctl cat failed); nothing was replaced"
  done
}

restart_units() {
  for unit in $UNITS; do
    remote "systemctl restart $(quote_arg "$unit")" ||
      die "systemctl restart $unit failed on $HOST"
  done
}

run_ship() {
  if [ "$NO_BUILD" = 0 ]; then
    build_services
  fi
  if [ "$DRY" = 0 ]; then
    check_binaries
  fi
  probe_host || die "cannot reach $HOST over ssh (check your ssh config and agent); nothing was changed"
  probe_units
  TS=$(date -u +%Y%m%dT%H%M%SZ)
  BACKUP="$BIN_DIR/apex-ship-backups/$TS"
  backup_cmd="mkdir -p $(quote_arg "$BACKUP")"
  for svc in $SERVICES; do
    backup_cmd="$backup_cmd && cp -a $(quote_arg "$BIN_DIR/$svc") $(quote_arg "$BACKUP/")"
  done
  remote "$backup_cmd" || die "remote backup into $BACKUP failed; nothing was replaced"
  for svc in $SERVICES; do
    if [ "$DRY" = 1 ]; then
      print_cmd rsync -a "$ART/$svc" "$HOST:$BIN_DIR/"
    else
      rsync -a "$ART/$svc" "$HOST:$BIN_DIR/" || die "rsync of $svc failed; restart skipped"
    fi
  done
  restart_units
  if [ "$DRY" = 1 ]; then
    printf '%s\n' "ship: dry run complete (nothing was executed)"
  else
    printf '%s\n' "ship: $SERVICES -> $HOST:$BIN_DIR"
    printf '%s\n' "ship: restarted: $UNITS"
    printf '%s\n' "ship: rollback with: sh tools/ci/ship.sh --rollback --host $HOST"
  fi
}

remote_ls_backup() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" \
    "ls -1d $(quote_arg "$BIN_DIR/apex-ship-backups")/*/ 2>/dev/null | sort | tail -n 1"
}

run_rollback() {
  probe_host || die "cannot reach $HOST over ssh (check your ssh config and agent); nothing was changed"
  if [ "$DRY" = 1 ]; then
    printf '%s\n' "ship: rollback plan: newest backup under $BIN_DIR/apex-ship-backups on $HOST"
    print_cmd ssh "$HOST" "ls -1d $(quote_arg "$BIN_DIR/apex-ship-backups")/*/ 2>/dev/null | sort | tail -n 1"
    for svc in $SERVICES; do
      print_cmd ssh "$HOST" "cp -a $(quote_arg "$BIN_DIR/apex-ship-backups/<newest>/$svc") $(quote_arg "$BIN_DIR/$svc")"
    done
    restart_units
    printf '%s\n' "ship: dry run complete (nothing was executed)"
    return 0
  fi
  PB=""
  if ! PB=$(remote_ls_backup); then
    die "cannot list backups under $BIN_DIR/apex-ship-backups on $HOST"
  fi
  PB="${PB%/}"
  [ -n "$PB" ] || die "no backup found under $BIN_DIR/apex-ship-backups on $HOST"
  for svc in $SERVICES; do
    remote "test -f $(quote_arg "$PB/$svc")" ||
      die "backup $PB is missing $svc; nothing was restored"
  done
  probe_units
  for svc in $SERVICES; do
    remote "cp -a $(quote_arg "$PB/$svc") $(quote_arg "$BIN_DIR/$svc")" ||
      die "restore of $svc from $PB failed"
  done
  restart_units
  printf '%s\n' "ship: rolled back $SERVICES from $PB"
  printf '%s\n' "ship: restarted: $UNITS"
}

if [ "$ROLLBACK" = 1 ]; then
  run_rollback
else
  run_ship
fi
