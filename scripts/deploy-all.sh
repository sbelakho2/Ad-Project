#!/bin/bash
#
# COMPREHENSIVE DEPLOYMENT SCRIPT
# ================================
# This script ensures thorough deployments with:
# - Full rebuilds (no stale artifacts)
# - Database migrations
# - Service restarts with verification
# - Health checks
#
# Usage: ./scripts/deploy-all.sh [component]
#   component: all | backend | console | website (default: all)
#

set -euo pipefail

# Configuration
# Hetzner production deployments must ONLY target the App Server.
# The Test Server is reserved for SDK harness/synthetic traffic and must not receive production deploys.
SERVER="46.62.252.96"
if [[ -n "${DEPLOY_SERVER:-}" && "${DEPLOY_SERVER}" != "$SERVER" ]]; then
    echo "[ERROR] Refusing to deploy to non-app server (DEPLOY_SERVER=${DEPLOY_SERVER})." >&2
    echo "[TIP] Deployments are restricted to Hetzner App Server (${SERVER})." >&2
    exit 2
fi
SSH_KEY="$HOME/.ssh/hetzner-db-mac"
DEPLOY_USER="root"
DEPLOY_PATH="/opt/adproject"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# SSH helper
ssh_cmd() {
    ssh -i "$SSH_KEY" -o StrictHostKeyChecking=no "$DEPLOY_USER@$SERVER" "$@"
}

# SCP helper
scp_cmd() {
    scp -i "$SSH_KEY" -o StrictHostKeyChecking=no "$@"
}

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Component to deploy
COMPONENT="${1:-all}"

# Timestamp for this deployment
DEPLOY_TIMESTAMP=$(date +%Y%m%d_%H%M%S)

log_info "======================================"
log_info "Starting deployment: $COMPONENT"
log_info "Timestamp: $DEPLOY_TIMESTAMP"
log_info "Target server: $SERVER (app server only)"
log_info "======================================"

#
# PRE-DEPLOYMENT CHECKS
#
preflight_checks() {
    log_info "Running preflight checks..."
    
    # Check SSH connectivity
    if ! ssh_cmd "echo 'SSH OK'" &>/dev/null; then
        log_error "Cannot connect to server via SSH"
        exit 1
    fi
    log_success "SSH connectivity OK"
    
    # Check if git repo is clean (warn if not)
    cd "$PROJECT_ROOT"
    if [[ -n $(git status --porcelain) ]]; then
        log_warn "Git working directory is not clean"
        log_warn "Uncommitted changes may not be deployed"
    fi
    
    # Check server disk space
    DISK_FREE=$(ssh_cmd "df -h /opt | tail -1 | awk '{print \$4}'")
    log_info "Server disk free: $DISK_FREE"
}

#
# BACKEND DEPLOYMENT
#
deploy_backend() {
    log_info "======================================"
    log_info "Deploying BACKEND"
    log_info "======================================"
    
    cd "$PROJECT_ROOT/backend"
    
    # 1. Install dependencies locally
    log_info "Installing dependencies..."
    npm ci --prefer-offline 2>/dev/null || npm install
    
    # 2. Run TypeScript compilation to catch errors
    log_info "Compiling TypeScript..."
    npm run build || {
        log_error "TypeScript compilation failed"
        exit 1
    }
    log_success "TypeScript compilation OK"
    
    # 3. Sync to server (excluding node_modules, using rsync)
    log_info "Syncing backend to server..."
    rsync -avz --delete \
        --exclude 'node_modules' \
        --exclude '.git' \
        --exclude '.env' \
        --exclude '.env.*' \
        --exclude 'logs/*' \
        --exclude 'coverage' \
        --exclude '.env.local' \
        -e "ssh -i $SSH_KEY" \
        "$PROJECT_ROOT/backend/" \
        "$DEPLOY_USER@$SERVER:$DEPLOY_PATH/backend/"
    
    # 4. Install server dependencies and rebuild
    log_info "Installing server dependencies..."
    ssh_cmd "cd $DEPLOY_PATH/backend && npm ci --prefer-offline 2>/dev/null || npm install"
    
    # 5. Run database migrations
    log_info "Running database migrations..."
    ssh_cmd "ROOT=\"$DEPLOY_PATH\"; if [[ -f \"$DEPLOY_PATH/.env\" ]]; then source \"$DEPLOY_PATH/scripts/ops/source_env.sh\" \"$DEPLOY_PATH/.env\" >/dev/null; fi; cd \"$DEPLOY_PATH/backend\" && (npm run migrate 2>/dev/null || npm run migrate:legacy 2>/dev/null || echo 'No migrations or migration script not found')"
    
    # 6. Rebuild TypeScript on server
    log_info "Building on server..."
    ssh_cmd "cd $DEPLOY_PATH/backend && npm run build"
    
    # 7. Restart PM2 process
    log_info "Restarting backend service..."
    ssh_cmd "pm2 restart backend || pm2 start $DEPLOY_PATH/backend/dist/src/index.js --name backend"
    
    # 8. Wait and verify
    sleep 3
    log_info "Verifying backend health..."
    if ssh_cmd "curl -sf http://127.0.0.1:8080/health" &>/dev/null; then
        log_success "Backend is healthy!"
    else
        log_error "Backend health check failed!"
        ssh_cmd "pm2 logs backend --lines 50 --nostream"
        exit 1
    fi
}

#
# CONSOLE DEPLOYMENT (Next.js)
#
deploy_console() {
    log_info "======================================"
    log_info "Deploying CONSOLE (Next.js)"
    log_info "======================================"
    
    cd "$PROJECT_ROOT/console"
    
    # 1. Clean previous builds to avoid stale artifacts
    log_info "Cleaning previous builds..."
    rm -rf .next out
    
    # 2. Install dependencies
    log_info "Installing dependencies..."
    npm ci --prefer-offline 2>/dev/null || npm install
    
    # 3. Copy production environment file if exists
    if [[ -f .env.production ]]; then
        log_info "Using .env.production"
    fi
    
    # 4. Build with NEXT_PUBLIC_* vars at build time
    log_info "Building Next.js with production environment..."
    export NEXT_PUBLIC_API_URL="https://api.apexmediation.ee"
    export NEXT_PUBLIC_WS_URL="wss://api.apexmediation.ee"
    export NEXT_PUBLIC_DOCS_URL="https://docs.apexmediation.ee"
    npm run build || {
        log_error "Next.js build failed"
        exit 1
    }
    log_success "Next.js build OK"
    
    # 5. Ensure standalone output exists
    if [[ ! -d ".next/standalone" ]]; then
        log_error "Standalone build not found. Check next.config.js has output: 'standalone'"
        exit 1
    fi
    
    # 6. Prepare standalone package
    log_info "Preparing standalone package..."
    STANDALONE_DIR=".next/standalone"
    
    # Copy static files
    cp -r .next/static "$STANDALONE_DIR/.next/"
    cp -r public "$STANDALONE_DIR/"

    # Include lockfile so we can do deterministic installs on the server.
    # Note: In this repo, Next.js standalone output does not include node_modules.
    if [[ -f "package-lock.json" ]]; then
        cp package-lock.json "$STANDALONE_DIR/"
    fi
    
    # 7. Create backup on server
    BACKUP_NAME="console_backup_$DEPLOY_TIMESTAMP"
    log_info "Creating backup: $BACKUP_NAME"
    ssh_cmd "[ -d $DEPLOY_PATH/console_release ] && cp -r $DEPLOY_PATH/console_release $DEPLOY_PATH/$BACKUP_NAME || true"
    
    # 8. Sync standalone to server
    log_info "Syncing console standalone to server..."
    ssh_cmd "rm -rf $DEPLOY_PATH/console_release_new"
    rsync -avz --delete \
        -e "ssh -i $SSH_KEY" \
        "$STANDALONE_DIR/" \
        "$DEPLOY_USER@$SERVER:$DEPLOY_PATH/console_release_new/"

    # Preserve server-side secrets/config that aren't part of the build output.
    log_info "Preserving console .env..."
    ssh_cmd "[ -f $DEPLOY_PATH/console_release/.env ] && cp $DEPLOY_PATH/console_release/.env $DEPLOY_PATH/console_release_new/.env || true"
    
    # 9. Install sharp for image optimization
    log_info "Installing production dependencies on server..."
    # Standalone output doesn't include app scripts; ignore lifecycle scripts to avoid postinstall failures.
    ssh_cmd "cd $DEPLOY_PATH/console_release_new && (npm ci --omit=dev --ignore-scripts --prefer-offline 2>/dev/null || npm ci --omit=dev --ignore-scripts || npm install --omit=dev --ignore-scripts)"

    log_info "Installing sharp for image optimization..."
    ssh_cmd "cd $DEPLOY_PATH/console_release_new && npm install sharp --no-save --platform=linux --arch=x64 2>/dev/null || true"
    
    # 10. Atomic swap
    log_info "Performing atomic swap..."
    ssh_cmd "rm -rf $DEPLOY_PATH/console_release_old && \
             [ -d $DEPLOY_PATH/console_release ] && mv $DEPLOY_PATH/console_release $DEPLOY_PATH/console_release_old || true && \
             mv $DEPLOY_PATH/console_release_new $DEPLOY_PATH/console_release"
    
    # 11. Restart PM2 process with environment
    log_info "Restarting console service..."
    ssh_cmd "pm2 delete console 2>/dev/null || true && \
             cd $DEPLOY_PATH/console_release && \
             PORT=3000 \
             NODE_ENV=production \
             NEXT_PUBLIC_API_URL=https://api.apexmediation.ee \
             NEXT_PUBLIC_WS_URL=wss://api.apexmediation.ee \
             pm2 start server.js --name console"
    
    # 12. Wait and verify
    sleep 5
    log_info "Verifying console health..."
    if ssh_cmd "curl -sf http://127.0.0.1:3000 -o /dev/null"; then
        log_success "Console is healthy!"
    else
        log_error "Console health check failed!"
        ssh_cmd "pm2 logs console --lines 50 --nostream"
        # Rollback
        log_warn "Rolling back..."
        ssh_cmd "[ -d $DEPLOY_PATH/console_release_old ] && \
                 rm -rf $DEPLOY_PATH/console_release && \
                 mv $DEPLOY_PATH/console_release_old $DEPLOY_PATH/console_release && \
                 pm2 restart console"
        exit 1
    fi
    
    # 13. Clean old backup
    ssh_cmd "rm -rf $DEPLOY_PATH/console_release_old"
    log_success "Console deployment complete!"
}

#
# WEBSITE DEPLOYMENT (Next.js static)
#
deploy_website() {
    log_info "======================================"
    log_info "Deploying WEBSITE"
    log_info "======================================"
    
    cd "$PROJECT_ROOT/website"
    
    # 1. Clean previous builds
    log_info "Cleaning previous builds..."
    rm -rf .next out
    
    # 2. Install dependencies
    log_info "Installing dependencies..."
    npm ci --prefer-offline 2>/dev/null || npm install
    
    # 3. Build
    log_info "Building website..."
    npm run build || {
        log_error "Website build failed"
        exit 1
    }
    log_success "Website build OK"

    # 4. Ensure standalone output exists
    if [[ ! -d ".next/standalone" ]]; then
        log_error "Standalone build not found. Check next.config.js has output: 'standalone'"
        exit 1
    fi

    # 5. Prepare standalone package
    log_info "Preparing standalone package..."
    STANDALONE_DIR=".next/standalone"
    cp -r .next/static "$STANDALONE_DIR/.next/"
    cp -r public "$STANDALONE_DIR/"
    if [[ -f "package-lock.json" ]]; then
        cp package-lock.json "$STANDALONE_DIR/"
    fi

    # 6. Create backup on server
    BACKUP_NAME="website_backup_$DEPLOY_TIMESTAMP"
    log_info "Creating backup: $BACKUP_NAME"
    ssh_cmd "[ -d $DEPLOY_PATH/website_release ] && cp -r $DEPLOY_PATH/website_release $DEPLOY_PATH/$BACKUP_NAME || true"

    # 7. Sync standalone to server
    log_info "Syncing website standalone to server..."
    ssh_cmd "rm -rf $DEPLOY_PATH/website_release_new"
    rsync -avz --delete \
        -e "ssh -i $SSH_KEY" \
        "$STANDALONE_DIR/" \
        "$DEPLOY_USER@$SERVER:$DEPLOY_PATH/website_release_new/"

    # Preserve server-side secrets/config that aren't part of the build output.
    log_info "Preserving website .env..."
    ssh_cmd "[ -f $DEPLOY_PATH/website_release/.env ] && cp $DEPLOY_PATH/website_release/.env $DEPLOY_PATH/website_release_new/.env || true"

    # 8. Atomic swap
    log_info "Performing atomic swap..."
    ssh_cmd "rm -rf $DEPLOY_PATH/website_release_old && \
             [ -d $DEPLOY_PATH/website_release ] && mv $DEPLOY_PATH/website_release $DEPLOY_PATH/website_release_old || true && \
             mv $DEPLOY_PATH/website_release_new $DEPLOY_PATH/website_release"

    # 9. Restart PM2 process with environment
    log_info "Restarting website service..."
    ssh_cmd "pm2 delete website 2>/dev/null || true && \
             cd $DEPLOY_PATH/website_release && \
             PORT=3002 \
             NODE_ENV=production \
             pm2 start server.js --name website"

    # 10. Wait and verify
    sleep 3
    log_info "Verifying website health..."
    if ssh_cmd "curl -sf http://127.0.0.1:3002 -o /dev/null"; then
        log_success "Website is healthy!"
    else
        log_error "Website health check failed!"
        ssh_cmd "pm2 logs website --lines 50 --nostream"
        # Rollback
        log_warn "Rolling back..."
        ssh_cmd "[ -d $DEPLOY_PATH/website_release_old ] && \
                 rm -rf $DEPLOY_PATH/website_release && \
                 mv $DEPLOY_PATH/website_release_old $DEPLOY_PATH/website_release && \
                 pm2 delete website 2>/dev/null || true && \
                 cd $DEPLOY_PATH/website_release && PORT=3002 NODE_ENV=production pm2 start server.js --name website"
        exit 1
    fi

    # 11. Clean old
    ssh_cmd "rm -rf $DEPLOY_PATH/website_release_old"
    log_success "Website deployment complete!"
}

#
# POST-DEPLOYMENT VERIFICATION
#
verify_deployment() {
    log_info "======================================"
    log_info "Running post-deployment verification"
    log_info "======================================"
    
    local failures=0
    
    # Check backend
    log_info "Checking backend API..."
    if ssh_cmd "curl -sf http://127.0.0.1:8080/health" &>/dev/null; then
        log_success "Backend API: OK"
    else
        log_error "Backend API: FAILED"
        ((failures++))
    fi
    
    # Check backend auth endpoint
    log_info "Checking auth endpoint..."
    if ssh_cmd "curl -sf -X POST http://127.0.0.1:8080/api/v1/auth/login -H 'Content-Type: application/json' -d '{\"email\":\"test@test.com\",\"password\":\"wrong\"}' 2>&1 | grep -q 'Invalid email'"; then
        log_success "Auth endpoint: OK"
    else
        log_warn "Auth endpoint: Response unexpected (may still be OK)"
    fi
    
    # Check console
    log_info "Checking console..."
    if ssh_cmd "curl -sf http://127.0.0.1:3000 -o /dev/null"; then
        log_success "Console: OK"
    else
        log_error "Console: FAILED"
        ((failures++))
    fi
    
    # Check external endpoints
    log_info "Checking external endpoints..."
    if curl -sf "https://api.apexmediation.ee/health" &>/dev/null; then
        log_success "External API (api.apexmediation.ee): OK"
    else
        log_warn "External API: FAILED (may be DNS/proxy issue)"
    fi
    
    if curl -sf "https://console.apexmediation.ee" -o /dev/null; then
        log_success "External Console (console.apexmediation.ee): OK"
    else
        log_warn "External Console: FAILED (may be DNS/proxy issue)"
    fi
    
    # Print PM2 status
    log_info "PM2 Process Status:"
    ssh_cmd "pm2 list"
    
    if [[ $failures -gt 0 ]]; then
        log_error "Deployment completed with $failures failures"
        exit 1
    fi
    
    log_success "======================================"
    log_success "Deployment completed successfully!"
    log_success "======================================"
}

#
# MAIN EXECUTION
#
main() {
    preflight_checks
    
    case "$COMPONENT" in
        all)
            deploy_backend
            deploy_console
            # deploy_website  # Uncomment if needed
            ;;
        backend)
            deploy_backend
            ;;
        console)
            deploy_console
            ;;
        website)
            deploy_website
            ;;
        *)
            log_error "Unknown component: $COMPONENT"
            log_info "Usage: $0 [all|backend|console|website]"
            exit 1
            ;;
    esac
    
    verify_deployment
}

main
