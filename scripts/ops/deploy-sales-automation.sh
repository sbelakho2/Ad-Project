#!/bin/bash
# deploy-sales-automation.sh
# Deploys the sales automation system to the Hetzner App Server
#
# Usage: ./scripts/ops/deploy-sales-automation.sh [--migrate] [--restart]
#
# Options:
#   --migrate   Run database migrations
#   --restart   Restart the sales-automation service

set -euo pipefail

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
APP_SERVER="46.62.252.96"
SSH_USER="root"
REMOTE_PATH="/opt/adproject"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Parse arguments
MIGRATE=false
RESTART=false
for arg in "$@"; do
  case $arg in
    --migrate) MIGRATE=true ;;
    --restart) RESTART=true ;;
    *) log_error "Unknown argument: $arg"; exit 1 ;;
  esac
done

# SSH wrapper
ssh_run() {
  ssh -o StrictHostKeyChecking=accept-new "${SSH_USER}@${APP_SERVER}" "$@"
}

# SCP wrapper
scp_to() {
  scp -o StrictHostKeyChecking=accept-new -r "$1" "${SSH_USER}@${APP_SERVER}:$2"
}

echo ""
echo "🚀 ApexMediation Sales Automation Deployment"
echo "============================================="
echo ""

# 1. Build backend locally
log_info "Building backend..."
cd "$PROJECT_ROOT/backend"
npm run build 2>/dev/null || {
  log_warn "npm run build failed, trying direct tsc..."
  npx tsc --build
}
log_info "Build complete"

# 2. Copy sales automation files
log_info "Copying sales automation files to app server..."

# Copy compiled CLI
scp_to "dist/src/scripts/salesAutomationCli.js" "${REMOTE_PATH}/backend/dist/src/scripts/"

# Copy service files
scp_to "dist/services/sales-automation" "${REMOTE_PATH}/backend/dist/services/"

# Copy systemd service file
scp_to "$PROJECT_ROOT/infrastructure/hetzner/sales-automation.service" "/etc/systemd/system/"

log_info "Files copied"

# 3. Run migrations if requested
if [ "$MIGRATE" = true ]; then
  log_info "Running database migrations..."
  
  # Copy migration file
  scp_to "$PROJECT_ROOT/backend/migrations/041_sales_automation_outbound.sql" "${REMOTE_PATH}/backend/migrations/"
  
  # Run migration
  ssh_run "cd ${REMOTE_PATH}/backend && \
    source ../.env && \
    psql \"\$DATABASE_URL\" -f migrations/041_sales_automation_outbound.sql"
  
  log_info "Migrations complete"
fi

# 4. Setup and restart service if requested
if [ "$RESTART" = true ]; then
  log_info "Setting up systemd service..."
  
  ssh_run "systemctl daemon-reload && \
    systemctl enable sales-automation && \
    systemctl restart sales-automation"
  
  # Wait and check status
  sleep 2
  ssh_run "systemctl status sales-automation --no-pager" || true
  
  log_info "Service restarted"
fi

# 5. Show status
echo ""
log_info "Deployment complete! Summary:"
echo "  - Backend built and copied"
[ "$MIGRATE" = true ] && echo "  - Database migrations applied"
[ "$RESTART" = true ] && echo "  - Service restarted"
echo ""

# Show how to check logs
echo "To check logs:"
echo "  ssh ${SSH_USER}@${APP_SERVER} journalctl -u sales-automation -f"
echo ""

# Show how to run manually
echo "To run manually:"
echo "  ssh ${SSH_USER}@${APP_SERVER} 'cd ${REMOTE_PATH}/backend && node dist/src/scripts/salesAutomationCli.js help'"
echo ""
