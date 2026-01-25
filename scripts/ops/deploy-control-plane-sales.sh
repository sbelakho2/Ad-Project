#!/usr/bin/env bash
#
# Deploy Control Plane with Sales Automation UI
# ==============================================
# This script deploys the updated control-plane with sales automation
# management UI. It also ensures RESEND_API_KEY is properly configured.
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

ssh_cmd() {
    bash "$SSH_SCRIPT" app -- "$1"
}

log_info "======================================"
log_info "Deploying Control Plane (Sales Automation)"
log_info "======================================"

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

# Ensure RESEND_API_KEY is in control-plane.env (copy from main .env if not present)
if ! grep -q "RESEND_API_KEY" /opt/adproject/control-plane.env; then
    RESEND_KEY=$(grep "RESEND_API_KEY" /opt/adproject/.env | head -1)
    if [ -n "$RESEND_KEY" ]; then
        echo "" >> /opt/adproject/control-plane.env
        echo "# Sales Automation Email (Resend)" >> /opt/adproject/control-plane.env
        echo "$RESEND_KEY" >> /opt/adproject/control-plane.env
        echo "[OK] Added RESEND_API_KEY to control-plane.env"
    else
        echo "[WARN] RESEND_API_KEY not found in main .env - emails will be disabled"
    fi
else
    echo "[OK] RESEND_API_KEY already configured"
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
log_success "Control Plane Sales Automation deployed!"
log_info "======================================"
log_info ""
log_info "Access at: https://admin.apexmediation.ee/admin"
log_info "Sales Automation UI is now available in the sidebar."
