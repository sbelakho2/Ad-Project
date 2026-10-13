#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

# SSH wrapper for Hetzner production servers
# Usage:
#   ./scripts/ops/hetzner_ssh.sh                           # SSH to app server (default)
#   ./scripts/ops/hetzner_ssh.sh app                       # SSH to app server
#   ./scripts/ops/hetzner_ssh.sh db                        # SSH to database server
#   ./scripts/ops/hetzner_ssh.sh test                      # SSH to test server (157.90.126.205)
#   ./scripts/ops/hetzner_ssh.sh app -- "<remote cmd>"     # Run command on app server
#   ./scripts/ops/hetzner_ssh.sh db -- "<remote cmd>"      # Run command on db server
#
# ============================================================================
# HETZNER INFRASTRUCTURE ARCHITECTURE
# ============================================================================
#
#   App Server: 46.62.252.96 (apexmediation.ee)
#     ├─ control-plane.service  (Port 4000) - Orchestrator, API
#     ├─ backend (pm2)          (Port 8080) - Main API
#     ├─ console (pm2)          (Port 3000) - Admin Console
#     ├─ website (pm2)          (Port 3001) - Public Website
#     └─ edge-tunnel.service    - SSH tunnel to Test Server:3500
#
#   Test Server: 157.90.126.205
#     └─ edge-worker.service    (Port 3500) - ML inference (LLM, Classifier)
#         ⚠️  ALL ML WORKLOADS GO HERE, NOT TO APP SERVER!
#
#   DB Server: 77.42.44.35
#     └─ PostgreSQL + PgBouncer
#
#   Domains: apexmediation.ee, console.apexmediation.ee, api.apexmediation.ee
#
# ============================================================================

APP_SERVER="46.62.252.96"
DB_SERVER="77.42.44.35"
TEST_SERVER="157.90.126.205"
KEY="${APEX_HETZNER_SSH_KEY:-$HOME/.ssh/hetzner-db-mac}"
SSH_BIN="${SSH_BIN:-/usr/bin/ssh}"

if [[ ! -x "$SSH_BIN" ]]; then
  echo "[err] ssh binary not found/executable at $SSH_BIN" >&2
  echo "[tip] Set SSH_BIN or install OpenSSH client" >&2
  exit 1
fi

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
    test)
      TARGET_HOST="$TEST_SERVER"
      TARGET_NAME="test"
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
  REMOTE_CMD="$*"
  # Force a predictable shell + PATH on the remote side.
  # Some hosts/users default to zsh with a restricted PATH in non-login shells,
  # which can cause "command not found" (e.g. systemctl).
  # Also unset BASH_ENV/ENV: non-interactive shells may source these and print noisy output.
  REMOTE_PREFIX='unset BASH_ENV ENV; export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"; '
  SSH_ARGS=(
    -i "$KEY"
    -o BatchMode=yes
    -o ConnectTimeout=10
    -o ServerAliveInterval=10
    -o ServerAliveCountMax=30
    -o StrictHostKeyChecking=accept-new
  )
  if [[ "$TARGET_NAME" == "db" ]]; then
    # Use an explicit ProxyCommand so the jump host always uses the same identity file.
    # (Some OpenSSH setups don't reliably apply -i to the ProxyJump connection.)
    SSH_ARGS+=(
      -o "ProxyCommand=$SSH_BIN -i $KEY -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=10 -o ServerAliveCountMax=30 -o StrictHostKeyChecking=accept-new root@$APP_SERVER -W %h:%p"
    )
  fi
  echo "[run] ssh ${SSH_ARGS[*]} root@$TARGET_HOST ($TARGET_NAME) -- /bin/bash --noprofile --norc -c <cmd>"
  "$SSH_BIN" "${SSH_ARGS[@]}" "root@$TARGET_HOST" -- /bin/bash --noprofile --norc -c "${REMOTE_PREFIX}${REMOTE_CMD}"
else
  SSH_ARGS=(
    -i "$KEY"
    -o BatchMode=yes
    -o ConnectTimeout=10
    -o ServerAliveInterval=10
    -o ServerAliveCountMax=30
    -o StrictHostKeyChecking=accept-new
  )
  if [[ "$TARGET_NAME" == "db" ]]; then
    SSH_ARGS+=(
      -o "ProxyCommand=$SSH_BIN -i $KEY -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=10 -o ServerAliveCountMax=30 -o StrictHostKeyChecking=accept-new root@$APP_SERVER -W %h:%p"
    )
  fi
  echo "[run] ssh ${SSH_ARGS[*]} root@$TARGET_HOST ($TARGET_NAME)"
  "$SSH_BIN" "${SSH_ARGS[@]}" "root@$TARGET_HOST"
fi
