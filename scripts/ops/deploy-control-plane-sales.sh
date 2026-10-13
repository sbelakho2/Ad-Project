#!/usr/bin/env bash
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
#
# Deploy Control Plane (Proxy Mode) to App Server
# ================================================
# This script deploys the control-plane to the App Server.
#
# ⚠️  ARCHITECTURE (January 2026):
# ⚠️  The Control Plane is now STRICTLY A PROXY CLIENT for sales automation.
# ⚠️  ALL sales automation work runs on the Test Server (157.90.126.205:4001).
# ⚠️  This script deploys the PROXY component to App Server only.
#
# For deploying sales-api to Test Server, use: deploy-sales-automation.sh
#
# Usage: ./scripts/ops/deploy-control-plane-sales.sh
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
SSH_SCRIPT="$PROJECT_ROOT/scripts/ops/hetzner_ssh.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# ⚠️  SERVER ARCHITECTURE (January 2026):
# ⚠️  - App Server (46.62.252.96): Control Plane (PROXY MODE), Console, Backend, Website
# ⚠️  - Test Server (157.90.126.205): sales-api (ALL sales automation), Edge Worker, inbound-smtp
# ⚠️  - DB Server (77.42.44.35): PostgreSQL (via App Server PgBouncer only)
# ⚠️
# ⚠️  This script deploys the PROXY component to App Server only!
# ⚠️  The control-plane forwards /api/autonomous/* to Test Server via SSH tunnel.
APP_SERVER_IP="46.62.252.96"
TEST_SERVER_IP="157.90.126.205"

ssh_cmd() {
    bash "$SSH_SCRIPT" app -- "$1"
}

log_info "======================================"
log_info "Deploying Control Plane (Sales Automation)"
log_info "======================================"

# Validate we're deploying to the correct server
log_info "Validating deployment target..."
ACTUAL_IP=$(ssh_cmd "hostname -I | awk '{print \$1}'" 2>/dev/null | tail -n 1 || echo "unknown")
if [[ "$ACTUAL_IP" != "$APP_SERVER_IP" ]]; then
  log_error "═══════════════════════════════════════════════════════════════"
  log_error "  ⛔ DEPLOYMENT TARGET MISMATCH!"
  log_error "═══════════════════════════════════════════════════════════════"
  log_error "  Expected: $APP_SERVER_IP (App Server)"
  log_error "  Actual:   $ACTUAL_IP"
  log_error ""
  log_error "  Control Plane must deploy to App Server only!"
  log_error "  ML workloads go to Test Server ($TEST_SERVER_IP)"
  log_error "═══════════════════════════════════════════════════════════════"
  exit 1
fi
log_success "Target validated: App Server ($APP_SERVER_IP)"

# Step 1: Build locally first
log_info "Building control-plane locally..."
cd "$PROJECT_ROOT/control-plane"
npm install
npm run build
log_success "Local build complete"

# Step 2: Create tarball of control-plane
log_info "Creating deployment package..."
cd "$PROJECT_ROOT"
export COPYFILE_DISABLE=1
export COPY_EXTENDED_ATTRIBUTES_DISABLE=1
tar czf /tmp/control-plane-deploy.tar.gz \
    --no-xattrs \
    --no-acls \
    --exclude='node_modules' \
    --exclude='.git' \
    --exclude='test-results' \
    --exclude='coverage' \
    control-plane/
unset COPY_EXTENDED_ATTRIBUTES_DISABLE
unset COPYFILE_DISABLE
log_success "Package created"

# Step 3: Upload to server
log_info "Uploading to server..."
scp -i "$HOME/.ssh/hetzner-db-mac" -o StrictHostKeyChecking=accept-new \
    /tmp/control-plane-deploy.tar.gz \
    root@46.62.252.96:/tmp/
log_success "Upload complete"

# Step 4: Deploy on server
log_info "Deploying on server..."
ssh_cmd '
set -e

# Backup current control-plane
if [ -d /opt/adproject/control-plane ]; then
    cp -r /opt/adproject/control-plane /opt/adproject/control-plane.backup.$(date +%Y%m%d_%H%M%S)
fi
ls -1dt /opt/adproject/control-plane.backup.* 2>/dev/null | tail -n +3 | xargs -r rm -rf

# Extract new version
cd /opt/adproject
tar xzf /tmp/control-plane-deploy.tar.gz
rm /tmp/control-plane-deploy.tar.gz

# Install dependencies
cd /opt/adproject/control-plane
NPM_CONFIG_LOGLEVEL=error npm ci --omit=dev --no-audit --no-fund

# Ensure ApexMail SMTP config is in control-plane.env (copy from main .env if not present)
if ! grep -q "SMTP_HOST" /opt/adproject/control-plane.env && ! grep -q "EMAIL_FROM" /opt/adproject/control-plane.env; then
    echo "" >> /opt/adproject/control-plane.env
    echo "# ApexMail - Self-hosted email (replaces Resend)" >> /opt/adproject/control-plane.env
    
    # Copy SMTP settings from main .env if present
    for VAR in SMTP_HOST SMTP_PORT SMTP_SECURE SMTP_USER SMTP_PASS EMAIL_FROM EMAIL_FROM_NAME APEXMAIL_API_KEY; do
        VAL=$(grep "^${VAR}=" /opt/adproject/.env | head -1)
        if [ -n "$VAL" ]; then
            echo "$VAL" >> /opt/adproject/control-plane.env
        fi
    done
    echo "[OK] Added ApexMail config to control-plane.env"
else
    echo "[OK] ApexMail config already present"
fi

# Restart service
systemctl restart control-plane
sleep 2

# Verify
if systemctl is-active --quiet control-plane; then
    echo "[OK] control-plane service is running"
else
    echo "[ERROR] control-plane service failed to start"
    journalctl -u control-plane -n 20 --no-pager
    exit 1
fi
'

log_success "Deployment complete!"

# Step 5: Verify API is responding
log_info "Verifying API health..."
sleep 3
HEALTH_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" https://admin.apexmediation.ee/health 2>/dev/null || echo "000")
if [ "$HEALTH_RESPONSE" = "200" ]; then
    log_success "Health check passed (HTTP $HEALTH_RESPONSE)"
else
    log_warn "Health check returned HTTP $HEALTH_RESPONSE (may still be starting)"
fi

log_info "======================================"
log_success "Control Plane (Proxy Mode) deployed!"
log_info "======================================"
log_info ""
log_info "Access at: https://admin.apexmediation.ee/admin"
log_info "Sales Automation UI is available in the sidebar."
log_info ""
log_info "⚠️  ARCHITECTURE REMINDER:"
log_info "  - Control Plane is PROXY MODE (forwards /api/autonomous/* to Test Server)"
log_info "  - sales-api runs on Test Server (157.90.126.205:4001)"
log_info "  - SSH tunnel: App Server localhost:4001 → Test Server:4001"
log_info ""
log_info "To deploy sales-api to Test Server: ./scripts/ops/deploy-sales-automation.sh"
