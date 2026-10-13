#!/bin/bash
# deploy-sales-automation.sh
# Deploys the sales automation system (Autonomous Sales V2) to the Hetzner Test Server
#
# ⚠️  ARCHITECTURE (January 2026):
# ⚠️  ALL sales automation runs on the TEST SERVER (157.90.126.205)
# ⚠️  The App Server control-plane is strictly a proxy client
#
# Usage: ./scripts/ops/deploy-sales-automation.sh [--migrate] [--restart]
#
# Options:
#   --migrate   Run database migrations (via db-tunnel)
#   --restart   Restart the sales-api PM2 process

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ⚠️  SERVER ARCHITECTURE (January 2026):
# ⚠️  - App Server (46.62.252.96): Backend, Console, Control Plane (PROXY ONLY)
# ⚠️  - Test Server (157.90.126.205): ALL Sales Automation, Edge Worker, Inbound SMTP
# ⚠️  - DB Server (77.42.44.35): PostgreSQL (via App Server PgBouncer only)
# ⚠️
# ⚠️  Sales Automation deploys to TEST SERVER!
APP_SERVER="46.62.252.96"
TEST_SERVER="157.90.126.205"  # ✅ Sales automation runs HERE
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

# SSH wrapper - connect to TEST SERVER
ssh_run() {
  ssh -o StrictHostKeyChecking=accept-new "${SSH_USER}@${TEST_SERVER}" "$@"
}

# SCP wrapper - copy to TEST SERVER
scp_to() {
  scp -o StrictHostKeyChecking=accept-new -r "$1" "${SSH_USER}@${TEST_SERVER}:$2"
}

echo ""
echo "🚀 ApexMediation Sales Automation Deployment (V2 - Test Server)"
echo "================================================================"
echo ""
echo "⚠️  Architecture: ALL sales automation runs on Test Server"
echo "⚠️  App Server control-plane is PROXY ONLY"
echo ""

# Validate deployment target
log_info "Validating deployment target (Test Server: $TEST_SERVER)..."
ACTUAL_IP=$(ssh_run "hostname -I | awk '{print \$1}'" 2>/dev/null || echo "unknown")
if [[ "$ACTUAL_IP" != "$TEST_SERVER" && "$ACTUAL_IP" != "157.90.126.205" ]]; then
  log_error "═══════════════════════════════════════════════════════════════"
  log_error "  ⛔ DEPLOYMENT TARGET MISMATCH!"
  log_error "═══════════════════════════════════════════════════════════════"
  log_error "  Expected: $TEST_SERVER (Test Server)"
  log_error "  Actual:   $ACTUAL_IP"
  log_error ""
  log_error "  Sales Automation V2 must deploy to Test Server!"
  log_error "  App Server ($APP_SERVER) is PROXY ONLY."
  log_error "═══════════════════════════════════════════════════════════════"
  exit 1
fi
log_info "Target validated: Test Server ($TEST_SERVER)"

# 1. Build control-plane locally (sales-api is part of control-plane)
log_info "Building control-plane..."
cd "$PROJECT_ROOT/control-plane"
npm run build 2>/dev/null || {
  log_warn "npm run build failed, trying direct tsc..."
  npx tsc --build
}
log_info "Build complete"

# 2. Copy control-plane files to Test Server
log_info "Copying control-plane files to Test Server..."

# Copy compiled dist
scp_to "dist" "${REMOTE_PATH}/control-plane/"

# Copy package files
scp_to "package.json" "${REMOTE_PATH}/control-plane/"
scp_to "package-lock.json" "${REMOTE_PATH}/control-plane/"

log_info "Files copied"

# 3. Install dependencies on Test Server
log_info "Installing dependencies on Test Server..."
ssh_run "cd ${REMOTE_PATH}/control-plane && npm ci --production"
log_info "Dependencies installed"

# 4. Run migrations if requested (via db-tunnel on Test Server)
if [ "$MIGRATE" = true ]; then
  log_info "Running database migrations via db-tunnel..."
  
  # Copy migration file
  scp_to "$PROJECT_ROOT/backend/migrations/041_sales_automation_outbound.sql" "${REMOTE_PATH}/backend/migrations/"
  
  # Run migration via db-tunnel (localhost:6432 → App Server → DB Server)
  ssh_run "cd ${REMOTE_PATH}/control-plane && \
    source .env && \
    psql 'postgres://adproject:\${POSTGRES_PASSWORD}@localhost:6432/adproject?sslmode=disable' \
      -f ../backend/migrations/041_sales_automation_outbound.sql"
  
  log_info "Migrations complete"
fi

# 5. Restart sales-api PM2 process if requested
if [ "$RESTART" = true ]; then
  log_info "Restarting sales-api PM2 process..."
  
  ssh_run "cd ${REMOTE_PATH}/control-plane && \
    pm2 restart sales-api || pm2 start dist/src/index.js --name sales-api"
  
  # Wait and check status
  sleep 2
  ssh_run "pm2 status"
  
  log_info "PM2 process restarted"
fi

# 6. Show status
echo ""
log_info "Deployment complete! Summary:"
echo "  - control-plane built and copied to Test Server"
[ "$MIGRATE" = true ] && echo "  - Database migrations applied (via db-tunnel)"
[ "$RESTART" = true ] && echo "  - sales-api PM2 process restarted"
echo ""

# Show how to check logs
echo "To check logs (Test Server):"
echo "  ./scripts/ops/hetzner_ssh.sh test -- 'pm2 logs sales-api'"
echo ""

# Show how to verify
echo "To verify (from local):"
echo "  ./scripts/ops/hetzner_ssh.sh test -- 'curl -s http://localhost:4001/api/autonomous/health | jq'"
echo ""

# Show architecture reminder
echo "⚠️  ARCHITECTURE REMINDER:"
echo "  - sales-api runs on Test Server (157.90.126.205:4001)"
echo "  - App Server control-plane is PROXY ONLY (forwards /api/autonomous/*)"
echo "  - SSH tunnel: App Server localhost:4001 → Test Server:4001"
echo ""
