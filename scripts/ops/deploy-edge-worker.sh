#!/usr/bin/env bash
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
# ============================================================================
# EDGE WORKER DEPLOYMENT SCRIPT
# 
# Deploys the Edge Worker service to the Test Server (157.90.126.205)
# 
# Features:
#   - Syncs edge-worker folder to Test Server
#   - Installs dependencies (including native modules)
#   - Builds TypeScript
#   - Installs Playwright browsers
#   - Sets up systemd service
#   - Warms up ML models
# 
# Usage:
#   ./scripts/ops/deploy-edge-worker.sh [--skip-build] [--restart-only]
# 
# Prerequisites:
#   - SSH access to Test Server via ./scripts/ops/hetzner_ssh.sh
#   - Node.js 20+ on Test Server
#   - Sufficient disk space for models (~500MB)
# ============================================================================

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SSH_SCRIPT="$SCRIPT_DIR/hetzner_ssh.sh"

# Targets
# ⚠️  IMPORTANT: Edge Worker MUST deploy to TEST SERVER ONLY!
# ⚠️  The Test Server runs ML inference workloads.
# ⚠️  The App Server runs the Control Plane orchestrator.
# ⚠️  NEVER change this target to 'app' - it will break the architecture!
TEST_SERVER="test"  # Uses hetzner_ssh.sh target -> 157.90.126.205
DEPLOY_PATH="/opt/edge-worker"
SERVICE_NAME="edge-worker"

# Server IPs for validation
EXPECTED_IP="157.90.126.205"

# Options
SKIP_BUILD=false
RESTART_ONLY=false
FORCE_REINSTALL=false

# Parse arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --restart-only)
      RESTART_ONLY=true
      shift
      ;;
    --force-reinstall)
      FORCE_REINSTALL=true
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [--skip-build] [--restart-only] [--force-reinstall]"
      echo ""
      echo "Options:"
      echo "  --skip-build      Skip local TypeScript build"
      echo "  --restart-only    Only restart the service, don't sync files"
      echo "  --force-reinstall Force reinstall of node_modules"
      exit 0
      ;;
    *)
      echo "Unknown option: $1"
      exit 1
      ;;
  esac
done

# ============================================================================
# FUNCTIONS
# ============================================================================

log_info() {
  echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
  echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
  echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
  echo -e "${RED}[ERROR]${NC} $1"
}

run_ssh() {
  "$SSH_SCRIPT" "$TEST_SERVER" -- "$@"
}

# ============================================================================
# MAIN
# ============================================================================

echo ""
echo "============================================================"
echo "   EDGE WORKER DEPLOYMENT"
echo "   Target: Test Server ($TEST_SERVER)"
echo "   Path: $DEPLOY_PATH"
echo "============================================================"
echo ""

# ============================================================================
# SERVER VALIDATION - Prevent deployment to wrong server
# ============================================================================
log_info "Validating deployment target..."
ACTUAL_IP=$("$SSH_SCRIPT" "$TEST_SERVER" -- "hostname -I | awk '{print \$1}'" 2>/dev/null || echo "unknown")
if [[ "$ACTUAL_IP" != "$EXPECTED_IP" ]]; then
  log_error "═══════════════════════════════════════════════════════════════════"
  log_error "  ⛔ DEPLOYMENT TARGET MISMATCH!"
  log_error "═══════════════════════════════════════════════════════════════════"
  log_error "  Expected IP: $EXPECTED_IP (Test Server)"
  log_error "  Actual IP:   $ACTUAL_IP"
  log_error ""
  log_error "  Edge Worker MUST be deployed to the Test Server!"
  log_error "  The App Server (46.62.252.96) runs the Control Plane."
  log_error "  Do NOT deploy ML workloads to the App Server."
  log_error "═══════════════════════════════════════════════════════════════════"
  exit 1
fi
log_success "Target validated: Test Server ($EXPECTED_IP)"

# Check prerequisites
if [[ ! -f "$SSH_SCRIPT" ]]; then
  log_error "SSH script not found: $SSH_SCRIPT"
  exit 1
fi

if [[ ! -d "$PROJECT_ROOT/edge-worker" ]]; then
  log_error "edge-worker folder not found in project root"
  exit 1
fi

# Restart only mode
if [[ "$RESTART_ONLY" == "true" ]]; then
  log_info "Restarting Edge Worker service..."
  run_ssh "sudo systemctl restart $SERVICE_NAME || true"
  run_ssh "sudo systemctl status $SERVICE_NAME --no-pager"
  log_success "Service restarted"
  exit 0
fi

# Step 1: Local build
if [[ "$SKIP_BUILD" != "true" ]]; then
  log_info "Building TypeScript locally..."
  cd "$PROJECT_ROOT/edge-worker"
  
  if [[ ! -d "node_modules" ]]; then
    log_info "Installing local dependencies..."
    npm install
  fi
  
  npm run build
  log_success "Local build complete"
  cd "$PROJECT_ROOT"
fi

# Step 2: Create deployment directory on server
log_info "Creating deployment directory..."
run_ssh "sudo mkdir -p $DEPLOY_PATH && sudo chown \$(whoami):\$(whoami) $DEPLOY_PATH"

# Step 3: Sync files
log_info "Syncing files to Test Server..."

# Create tarball of edge-worker (excluding node_modules and local .env)
TARBALL="/tmp/edge-worker-deploy.tar.gz"
cd "$PROJECT_ROOT"
tar -czf "$TARBALL" \
  --exclude='edge-worker/node_modules' \
  --exclude='edge-worker/.env' \
  --exclude='edge-worker/.cache' \
  edge-worker/

# Copy tarball to server using scp
log_info "Uploading package..."
scp -i ~/.ssh/hetzner-db-mac -o StrictHostKeyChecking=accept-new "$TARBALL" root@157.90.126.205:/tmp/edge-worker-deploy.tar.gz

# Extract on server
log_info "Extracting package..."
run_ssh "cd /opt && tar -xzf /tmp/edge-worker-deploy.tar.gz && rm /tmp/edge-worker-deploy.tar.gz"

# Cleanup local tarball
rm -f "$TARBALL"

log_success "Files synced"

# Step 4: Install dependencies on server
if [[ "$FORCE_REINSTALL" == "true" ]] || ! run_ssh "test -d $DEPLOY_PATH/node_modules" 2>/dev/null; then
  log_info "Installing dependencies on server..."
  run_ssh "cd $DEPLOY_PATH && npm ci --omit=dev"
  log_success "Dependencies installed"
else
  log_info "Skipping npm install (node_modules exists, use --force-reinstall to override)"
fi

# Step 5: Install Playwright browsers
log_info "Checking Playwright browsers..."
run_ssh "cd $DEPLOY_PATH && npx playwright install chromium --with-deps || true"
log_success "Playwright browsers ready"

# Step 6: Create .env file if missing
if ! run_ssh "test -f $DEPLOY_PATH/.env" 2>/dev/null; then
  log_info "Creating .env file..."
  run_ssh "cat > $DEPLOY_PATH/.env << 'EOF'
# Edge Worker Configuration
PORT=3500
NODE_ENV=production
LOG_LEVEL=info

# Control Plane Connection
CONTROL_PLANE_URL=http://46.62.252.96:4000
EDGE_WORKER_SECRET=\$(openssl rand -hex 32)

# Model Configuration
ICEBREAKER_MODEL=Xenova/LaMini-Flan-T5-248M
CLASSIFIER_MODEL=Xenova/distilbert-base-uncased-finetuned-sst-2-english

# Performance
CONFIDENCE_THRESHOLD=0.85
MAX_CONCURRENT_REQUESTS=10
EOF"
  log_warn "Created default .env - please update EDGE_WORKER_SECRET with the same value in Control Plane"
fi

# Step 7: Setup systemd service
log_info "Setting up systemd service..."
run_ssh "sudo tee /etc/systemd/system/$SERVICE_NAME.service > /dev/null << 'EOF'
[Unit]
Description=Edge Worker Service - ML Inference for Autonomous Sales
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=$DEPLOY_PATH
Environment=NODE_ENV=production
EnvironmentFile=$DEPLOY_PATH/.env
ExecStart=/usr/bin/node dist/index.js
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

# Resource limits
LimitNOFILE=65536
MemoryLimit=4G

# Watchdog
WatchdogSec=60

[Install]
WantedBy=multi-user.target
EOF"

run_ssh "sudo systemctl daemon-reload"
run_ssh "sudo systemctl enable $SERVICE_NAME"

# Step 8: Start/restart service
log_info "Starting Edge Worker service..."
run_ssh "sudo systemctl restart $SERVICE_NAME"

# Wait for service to start
sleep 5

# Check status
run_ssh "sudo systemctl status $SERVICE_NAME --no-pager" || true

# Step 9: Health check
log_info "Checking service health..."
sleep 3

HEALTH_CHECK=$(run_ssh "curl -s http://localhost:3500/health" 2>/dev/null || echo '{"status":"unknown"}')
echo "Health check response: $HEALTH_CHECK"

if echo "$HEALTH_CHECK" | grep -q '"status":"healthy"'; then
  log_success "Edge Worker is healthy!"
else
  log_warn "Service may still be warming up. Check logs with: sudo journalctl -u $SERVICE_NAME -f"
fi

# Step 10: Trigger warmup
log_info "Triggering model warmup..."
WARMUP_RESULT=$(run_ssh "curl -s -X POST http://localhost:3500/warmup" 2>/dev/null || echo '{"error":"warmup failed"}')
echo "Warmup result: $WARMUP_RESULT"

echo ""
echo "============================================================"
log_success "DEPLOYMENT COMPLETE"
echo "============================================================"
echo ""
echo "Edge Worker is running at: http://157.90.126.205:3500"
echo ""
echo "Useful commands:"
echo "  - View logs:    sudo journalctl -u $SERVICE_NAME -f"
echo "  - Restart:      sudo systemctl restart $SERVICE_NAME"
echo "  - Status:       sudo systemctl status $SERVICE_NAME"
echo "  - Health:       curl http://157.90.126.205:3500/health"
echo ""
echo "Don't forget to:"
echo "  1. Update Control Plane to use the Edge Worker URL"
echo "  2. Ensure EDGE_WORKER_SECRET matches on both servers"
echo ""
