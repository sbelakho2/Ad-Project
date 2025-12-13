#!/usr/bin/env bash
set -euo pipefail

# SSH wrapper for Hetzner production servers
# Usage:
#   ./scripts/ops/hetzner_ssh.sh                           # SSH to app server (default)
#   ./scripts/ops/hetzner_ssh.sh app                       # SSH to app server
#   ./scripts/ops/hetzner_ssh.sh db                        # SSH to database server
#   ./scripts/ops/hetzner_ssh.sh app -- "<remote cmd>"     # Run command on app server
#   ./scripts/ops/hetzner_ssh.sh db -- "<remote cmd>"      # Run command on db server
#
# Hetzner Infrastructure:
#   App Server: 46.62.252.96 (apexmediation.ee)
#   DB Server:  77.42.44.35
#   Domains: apexmediation.ee, console.apexmediation.ee, api.apexmediation.ee

APP_SERVER="46.62.252.96"
DB_SERVER="77.42.44.35"
KEY="${APEX_HETZNER_SSH_KEY:-$HOME/.ssh/hetzner-db-mac}"

if [[ ! -f "$KEY" ]]; then
  echo "[err] SSH key not found at $KEY" >&2
  echo "[tip] Set APEX_HETZNER_SSH_KEY or ensure ~/.ssh/hetzner-db-mac exists" >&2
  exit 1
fi

# Determine target server
TARGET_HOST="$APP_SERVER"
TARGET_NAME="app"

if [[ $# -gt 0 ]]; then
  case "$1" in
    app)
      TARGET_HOST="$APP_SERVER"
      TARGET_NAME="app"
      shift
      ;;
    db)
      TARGET_HOST="$DB_SERVER"
      TARGET_NAME="db"
      shift
      ;;
    --)
      # No server specified, use default (app)
      ;;
    *)
      # First arg is not a server name, treat as command
      ;;
  esac
fi

if [[ $# -gt 0 ]]; then
  # If first arg is --, shift and treat remainder as remote command
  if [[ "$1" == "--" ]]; then shift; fi
  echo "[run] ssh -i $KEY root@$TARGET_HOST ($TARGET_NAME) -- $*"
  ssh -i "$KEY" "root@$TARGET_HOST" -- "$@"
else
  echo "[run] ssh -i $KEY root@$TARGET_HOST ($TARGET_NAME)"
  ssh -i "$KEY" "root@$TARGET_HOST"
fi
