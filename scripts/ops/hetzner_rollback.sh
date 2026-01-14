#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)

usage() {
  cat <<'EOF'
Usage:
  ./scripts/ops/hetzner_rollback.sh --service backend|console|website|control-plane|all --ref <git-ref>

Notes:
- Requires git checkout to be available on /opt/adproject on the app server.
- Runs build + restart for the selected service(s).
EOF
}

SERVICE=""
REF=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --service)
      SERVICE=${2:-}
      shift 2
      ;;
    --ref)
      REF=${2:-}
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "[err] Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$SERVICE" || -z "$REF" ]]; then
  echo "[err] --service and --ref are required" >&2
  usage
  exit 2
fi

remote_cmd=$(cat <<'BASH'
set -euo pipefail
SERVICE="__SERVICE__"
REF="__REF__"

cd /opt/adproject

git fetch --all --tags
git checkout "$REF"

case "$SERVICE" in
  backend)
    cd /opt/adproject/backend
    npm ci
    npm run build
    pm2 restart backend
    ;;
  console)
    cd /opt/adproject/console
    npm ci
    npm run build
    pm2 restart console
    ;;
  website)
    cd /opt/adproject/website
    npm ci
    npm run build
    pm2 restart website
    ;;
  control-plane)
    cd /opt/adproject/control-plane
    npm ci
    npm run build
    systemctl restart control-plane
    ;;
  all)
    cd /opt/adproject/backend
    npm ci
    npm run build
    pm2 restart backend

    cd /opt/adproject/console
    npm ci
    npm run build
    pm2 restart console

    cd /opt/adproject/website
    npm ci
    npm run build
    pm2 restart website
    ;;
  *)
    echo "[err] unknown service: $SERVICE" >&2
    exit 2
    ;;
esac
BASH
)

remote_cmd=${remote_cmd/__SERVICE__/$SERVICE}
remote_cmd=${remote_cmd/__REF__/$REF}

"$ROOT_DIR/scripts/ops/hetzner_ssh.sh" app -- "$remote_cmd"
