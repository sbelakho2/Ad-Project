#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/ops/prod_analytics_rollups.sh start --start YYYY-MM-DD --end YYYY-MM-DD [--fg]
  ./scripts/ops/prod_analytics_rollups.sh tail
  ./scripts/ops/prod_analytics_rollups.sh status
  ./scripts/ops/prod_analytics_rollups.sh ps
  ./scripts/ops/prod_analytics_rollups.sh stop

Notes:
- Default is background (detached) with a server-side log file.
- A singleton lock prevents duplicate concurrent rollups.
- The log path is recorded on the server so `tail/status` work without args.
EOF
}

require_arg() {
  local name="$1"
  local value="$2"
  if [[ -z "$value" ]]; then
    echo "[err] missing required arg: $name" >&2
    usage
    exit 2
  fi
}

CMD=${1:-help}
shift || true

START_DATE=""
END_DATE=""
FOREGROUND=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --start)
      START_DATE=${2:-}
      shift 2
      ;;
    --end)
      END_DATE=${2:-}
      shift 2
      ;;
    --fg|--foreground)
      FOREGROUND=true
      shift
      ;;
    -h|--help|help)
      CMD=help
      shift
      ;;
    *)
      echo "[err] unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

remote_start() {
  require_arg --start "$START_DATE"
  require_arg --end "$END_DATE"

  local remote_cmd
  remote_cmd=$(cat <<'EOF'
set -euo pipefail

START_DATE='__START_DATE__'
END_DATE='__END_DATE__'
FOREGROUND='__FOREGROUND__'

cd /opt/adproject/backend

LOCK=/tmp/adproject-analytics-rollups.lock
PID=/tmp/adproject-analytics-rollups.pid
CURRENT=/tmp/adproject-analytics-rollups.current

# Generate a unique log per run, but keep a stable pointer in CURRENT.
ts=
if date -u '+%Y%m%dT%H%M%SZ' >/dev/null 2>&1; then ts=$(date -u '+%Y%m%dT%H%M%SZ'); else ts=$(date '+%Y%m%dT%H%M%SZ'); fi
LOG="/tmp/adproject-analytics-rollups-${ts}.log"

# Hard guard against accidental duplicate runs (even if they weren't started via this helper).
# Anchor on "node" to avoid false positives on wrapper bash command lines.
if pgrep -af "^node .*dist/src/scripts/analyticsRollupsCli\\.js" >/dev/null 2>&1; then
  echo "[err] another analytics rollups process is already running:" >&2
  pgrep -af "^node .*dist/src/scripts/analyticsRollupsCli\\.js" >&2 || true
  echo "[tip] use: ./scripts/ops/prod_analytics_rollups.sh ps" >&2
  exit 3
fi

if [[ "$FOREGROUND" == "true" ]]; then
  echo "[run] foreground rollups (log: $LOG)"
  if command -v flock >/dev/null 2>&1; then
    # `tee` keeps output streaming to SSH and the log.
    flock -n "$LOCK" -c "NODE_ENV=production node dist/src/scripts/analyticsRollupsCli.js --start '$START_DATE' --end '$END_DATE'" 2>&1 | tee -a "$LOG"
  else
    echo "[warn] flock not available; proceeding without singleton lock" >&2
    NODE_ENV=production node dist/src/scripts/analyticsRollupsCli.js --start "$START_DATE" --end "$END_DATE" 2>&1 | tee -a "$LOG"
  fi
  exit 0
fi

# Background mode: avoid SSH session issues and record pid + log path.
if [[ -f "$PID" ]] && kill -0 "$(cat "$PID" 2>/dev/null || true)" 2>/dev/null; then
  echo "[err] rollups already running (pid=$(cat "$PID"), log=$(cat "$CURRENT" 2>/dev/null || true))" >&2
  exit 3
fi

nohup bash -lc "set -euo pipefail; cd /opt/adproject/backend; \
  if command -v flock >/dev/null 2>&1; then \
    flock -n \"$LOCK\" -c \"NODE_ENV=production node dist/src/scripts/analyticsRollupsCli.js --start '$START_DATE' --end '$END_DATE'\"; \
  else \
    NODE_ENV=production node dist/src/scripts/analyticsRollupsCli.js --start '$START_DATE' --end '$END_DATE'; \
  fi" >>"$LOG" 2>&1 &
echo $! >"$PID"
printf '%s\n' "$LOG" >"$CURRENT"

echo "[ok] started rollups"
echo "[pid] $(cat "$PID")"
echo "[log] $LOG"
EOF
)

  remote_cmd=${remote_cmd/__START_DATE__/$START_DATE}
  remote_cmd=${remote_cmd/__END_DATE__/$END_DATE}
  remote_cmd=${remote_cmd/__FOREGROUND__/$FOREGROUND}

./scripts/ops/hetzner_ssh.sh app -- "$remote_cmd"
}

remote_tail() {
  ./scripts/ops/hetzner_ssh.sh app -- 'set -euo pipefail; CURRENT=/tmp/adproject-analytics-rollups.current; if [[ ! -f "$CURRENT" ]]; then echo "[err] no current rollups log recorded" >&2; exit 2; fi; LOG="$(cat "$CURRENT")"; echo "[tail] $LOG"; tail -n 200 -f "$LOG"'
}

remote_status() {
  ./scripts/ops/hetzner_ssh.sh app -- 'set -euo pipefail; PID=/tmp/adproject-analytics-rollups.pid; CURRENT=/tmp/adproject-analytics-rollups.current; if [[ -f "$PID" ]]; then p="$(cat "$PID" 2>/dev/null || true)"; else p=""; fi; if [[ -f "$CURRENT" ]]; then log="$(cat "$CURRENT" 2>/dev/null || true)"; else log=""; fi; if pgrep -af "^node .*dist/src/scripts/analyticsRollupsCli\\.js" >/dev/null 2>&1; then echo "[ok] running"; else echo "[ok] not running"; if [[ -n "$p" ]] && ! kill -0 "$p" 2>/dev/null; then rm -f "$PID" 2>/dev/null || true; fi; fi; echo "[rollups node ps]"; pgrep -af "^node .*dist/src/scripts/analyticsRollupsCli\\.js" || true; if [[ -n "$log" ]] && [[ -f "$log" ]]; then echo; echo "[log] $log"; tail -n 120 "$log" || true; fi'
}

remote_ps() {
  ./scripts/ops/hetzner_ssh.sh app -- 'set -euo pipefail; echo "[rollups node ps]"; pgrep -af "^node .*dist/src/scripts/analyticsRollupsCli\\.js" || true; echo; echo "[pidfile]"; ls -la /tmp/adproject-analytics-rollups.pid /tmp/adproject-analytics-rollups.current 2>/dev/null || true; if [[ -f /tmp/adproject-analytics-rollups.current ]]; then echo; echo "[current log tail]"; tail -n 80 "$(cat /tmp/adproject-analytics-rollups.current)" 2>/dev/null || true; fi'
}

remote_stop() {
  ./scripts/ops/hetzner_ssh.sh app -- 'set -euo pipefail; PID=/tmp/adproject-analytics-rollups.pid; if [[ ! -f "$PID" ]]; then echo "[ok] no pidfile present"; exit 0; fi; p="$(cat "$PID" 2>/dev/null || true)"; if [[ -z "$p" ]]; then echo "[ok] empty pidfile; removing"; rm -f "$PID"; exit 0; fi; if kill -0 "$p" 2>/dev/null; then echo "[run] stopping pid=$p (process group)"; pgid="$(ps -o pgid= -p "$p" 2>/dev/null | tr -d " ")"; if [[ -n "$pgid" ]]; then kill -TERM "-$pgid" 2>/dev/null || true; else kill -TERM "$p" 2>/dev/null || true; fi; sleep 2; if kill -0 "$p" 2>/dev/null; then echo "[warn] still running; sending SIGKILL"; if [[ -n "$pgid" ]]; then kill -KILL "-$pgid" 2>/dev/null || true; else kill -KILL "$p" 2>/dev/null || true; fi; fi; else echo "[ok] stale pidfile; removing"; fi; rm -f "$PID"'
}

case "$CMD" in
  start)
    remote_start
    ;;
  tail)
    remote_tail
    ;;
  status)
    remote_status
    ;;
  ps)
    remote_ps
    ;;
  stop)
    remote_stop
    ;;
  help|-h|--help)
    usage
    ;;
  *)
    echo "[err] unknown command: $CMD" >&2
    usage
    exit 2
    ;;
esac
