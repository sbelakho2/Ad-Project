#!/usr/bin/env bash
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
# =============================================================================
# ApexMediation Declarative Deployment Script
# =============================================================================
#
# This script reads from deployment.toml and applies the desired state.
# It is idempotent - running it multiple times produces the same result.
#
# Usage:
#   ./deploy.sh [component] [--dry-run]
#
# Components:
#   all         - Deploy all components
#   backend     - Deploy backend service only
#   console     - Deploy console service only
#   control-plane - Deploy control plane (systemd)
#
# Options:
#   --dry-run   - Show what would be done without making changes
#
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="${SCRIPT_DIR}/deployment.toml"
DRY_RUN=false
COMPONENT="${1:-all}"

# Parse arguments
for arg in "$@"; do
  case $arg in
    --dry-run)
      DRY_RUN=true
      shift
      ;;
  esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
  echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
  echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
  echo -e "${RED}[ERROR]${NC} $1"
}

# Simple TOML parser (reads key = "value" patterns)
parse_toml() {
  local key=$1
  grep -E "^${key}\s*=" "$CONFIG_FILE" | sed -E 's/^[^=]+=\s*"?([^"]*)"?/\1/' | tr -d '"'
}

# Parse nested TOML section values
parse_toml_section() {
  local section=$1
  local key=$2
  awk -v section="[$section]" -v key="$key" '
    $0 == section { in_section=1; next }
    /^\[/ { in_section=0 }
    in_section && $0 ~ "^" key " *=" { gsub(/^[^=]+= *"?|"?$/, ""); print }
  ' "$CONFIG_FILE"
}

# Check if config file exists
if [[ ! -f "$CONFIG_FILE" ]]; then
  log_error "Configuration file not found: $CONFIG_FILE"
  exit 1
fi

# Read configuration
APP_SERVER_IP=$(parse_toml "app_server_ip")
APP_SERVER_USER=$(parse_toml "app_server_ssh_user")
DEPLOY_PATH=$(parse_toml "app_server_deploy_path")

log_info "Deployment Configuration"
log_info "  App Server: ${APP_SERVER_USER}@${APP_SERVER_IP}"
log_info "  Deploy Path: ${DEPLOY_PATH}"
log_info "  Component: ${COMPONENT}"
log_info "  Dry Run: ${DRY_RUN}"
echo ""

# Function to execute command (respects dry-run)
run_cmd() {
  if [[ "$DRY_RUN" == "true" ]]; then
    log_info "[DRY-RUN] Would execute: $*"
  else
    "$@"
  fi
}

# Function to execute SSH command
ssh_cmd() {
  local cmd="$1"
  if [[ "$DRY_RUN" == "true" ]]; then
    log_info "[DRY-RUN] Would SSH: $cmd"
  else
    ssh -o StrictHostKeyChecking=no "${APP_SERVER_USER}@${APP_SERVER_IP}" "$cmd"
  fi
}

# Deploy backend service
deploy_backend() {
  log_info "Deploying backend service..."
  
  # Check PM2 status
  local pm2_status
  pm2_status=$(ssh_cmd "pm2 jlist 2>/dev/null | jq -r '.[] | select(.name==\"backend\") | .pm2_env.status' || echo 'not-running'")
  
  if [[ "$pm2_status" == "online" ]]; then
    log_info "Backend is running, performing rolling restart..."
    ssh_cmd "cd ${DEPLOY_PATH}/backend && npm run build && pm2 reload backend --update-env"
  else
    log_warn "Backend is not running, starting fresh..."
    ssh_cmd "cd ${DEPLOY_PATH}/backend && npm run build && pm2 start ecosystem.config.cjs --only backend"
  fi
  
  # Verify deployment
  if [[ "$DRY_RUN" != "true" ]]; then
    sleep 5
    local health
    health=$(ssh_cmd "curl -sf http://localhost:8080/health | jq -r '.status' || echo 'unhealthy'")
    if [[ "$health" == "healthy" ]]; then
      log_info "✓ Backend deployment successful"
    else
      log_error "✗ Backend health check failed"
      exit 1
    fi
  fi
}

# Deploy console service
deploy_console() {
  log_info "Deploying console service..."
  
  # Build Next.js app
  ssh_cmd "cd ${DEPLOY_PATH}/console && npm run build"
  
  # Copy standalone build to run directory
  ssh_cmd "rm -rf ${DEPLOY_PATH}/console-run && mkdir -p ${DEPLOY_PATH}/console-run"
  ssh_cmd "cp -r ${DEPLOY_PATH}/console/.next/standalone/* ${DEPLOY_PATH}/console-run/"
  ssh_cmd "cp -r ${DEPLOY_PATH}/console/.next/static ${DEPLOY_PATH}/console-run/.next/"
  ssh_cmd "cp -r ${DEPLOY_PATH}/console/public ${DEPLOY_PATH}/console-run/"
  
  # Restart PM2
  local pm2_status
  pm2_status=$(ssh_cmd "pm2 jlist 2>/dev/null | jq -r '.[] | select(.name==\"console\") | .pm2_env.status' || echo 'not-running'")
  
  if [[ "$pm2_status" == "online" ]]; then
    ssh_cmd "pm2 reload console --update-env"
  else
    ssh_cmd "pm2 start ${DEPLOY_PATH}/console-run/server.js --name console --cwd ${DEPLOY_PATH}/console-run"
  fi
  
  log_info "✓ Console deployment initiated"
}

# Deploy control plane (systemd service)
deploy_control_plane() {
  log_info "Deploying control plane (systemd)..."
  
  # Build
  ssh_cmd "cd ${DEPLOY_PATH}/control-plane && npm run build"
  
  # Check if service file exists
  local service_exists
  service_exists=$(ssh_cmd "test -f /etc/systemd/system/control-plane.service && echo 'yes' || echo 'no'")
  
  if [[ "$service_exists" == "no" ]]; then
    log_info "Installing systemd service file..."
    ssh_cmd "cp ${DEPLOY_PATH}/infrastructure/hetzner/control-plane.service /etc/systemd/system/"
    ssh_cmd "systemctl daemon-reload"
    ssh_cmd "systemctl enable control-plane"
  fi
  
  # Restart service
  ssh_cmd "systemctl restart control-plane"
  
  # Verify
  if [[ "$DRY_RUN" != "true" ]]; then
    sleep 3
    local status
    status=$(ssh_cmd "systemctl is-active control-plane || true")
    if [[ "$status" == "active" ]]; then
      log_info "✓ Control plane deployment successful"
    else
      log_error "✗ Control plane failed to start"
      ssh_cmd "journalctl -u control-plane -n 20 --no-pager" || true
      exit 1
    fi
  fi
}

# Verify all services are healthy
verify_all() {
  log_info "Verifying all services..."
  
  # PM2 services
  ssh_cmd "pm2 status"
  
  # Systemd services
  ssh_cmd "systemctl status control-plane --no-pager || true"
  ssh_cmd "systemctl status redis-server --no-pager || true"
  
  # Health checks
  log_info "Running health checks..."
  ssh_cmd "curl -sf http://localhost:8080/health && echo ' Backend OK' || echo 'Backend FAIL'"
  ssh_cmd "curl -sf http://localhost:3000 > /dev/null && echo 'Console OK' || echo 'Console FAIL'"
  ssh_cmd "curl -sf http://localhost:4000/health && echo 'Control Plane OK' || echo 'Control Plane FAIL'" || true
}

# Main execution
case "$COMPONENT" in
  backend)
    deploy_backend
    ;;
  console)
    deploy_console
    ;;
  control-plane)
    deploy_control_plane
    ;;
  all)
    deploy_backend
    deploy_console
    deploy_control_plane
    verify_all
    ;;
  verify)
    verify_all
    ;;
  *)
    log_error "Unknown component: $COMPONENT"
    echo "Usage: $0 [all|backend|console|control-plane|verify] [--dry-run]"
    exit 1
    ;;
esac

log_info "Deployment complete!"
