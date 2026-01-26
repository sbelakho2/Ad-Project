#!/usr/bin/env bash
#
# WORKER STARTUP SCRIPT
#
# Ensures all dependencies are available before starting the worker daemon.
# This includes:
#   - Node.js runtime
#   - Playwright browser binaries (chromium)
#   - ONNX models for local ML inference
#
# Usage:
#   ./scripts/start-worker.sh [--dry-run]
#
# This script should be called by systemd (sales-automation.service)
# or run manually on the test server (157.90.126.205)
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${SCRIPT_DIR}/.."

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
  log_info "Dry run mode - no changes will be made"
fi

# ============================================================================
# 1. CHECK NODE.JS
# ============================================================================

log_info "Checking Node.js installation..."

if ! command -v node &> /dev/null; then
  log_error "Node.js is not installed. Please install Node.js 20+."
  exit 1
fi

NODE_VERSION=$(node -v | sed 's/v//' | cut -d'.' -f1)
if [[ "$NODE_VERSION" -lt 20 ]]; then
  log_error "Node.js version 20+ required. Current: $(node -v)"
  exit 1
fi

log_info "Node.js $(node -v) ✓"

# ============================================================================
# 2. ENSURE PLAYWRIGHT BROWSERS
# ============================================================================

log_info "Checking Playwright browser installation..."

PLAYWRIGHT_BROWSERS="${PLAYWRIGHT_BROWSERS_PATH:-$HOME/.cache/ms-playwright}"

# Check if chromium is installed
if [[ ! -d "$PLAYWRIGHT_BROWSERS" ]] || [[ -z "$(ls -A "$PLAYWRIGHT_BROWSERS" 2>/dev/null | grep -i chromium || true)" ]]; then
  log_warn "Playwright browsers not found. Installing chromium..."
  
  if [[ "$DRY_RUN" == "false" ]]; then
    cd "$PROJECT_ROOT"
    
    # Install playwright browsers with system dependencies
    # --with-deps installs system libraries needed for headless Chrome
    npx playwright install chromium --with-deps 2>&1 | while read -r line; do
      echo "  $line"
    done
    
    log_info "Playwright chromium installed successfully ✓"
  else
    log_info "[DRY RUN] Would install: npx playwright install chromium --with-deps"
  fi
else
  log_info "Playwright browsers found at $PLAYWRIGHT_BROWSERS ✓"
fi

# ============================================================================
# 3. VALIDATE PLAYWRIGHT LAUNCH (Self-Healing Test)
# ============================================================================

log_info "Validating Playwright browser can launch..."

PLAYWRIGHT_TEST_SCRIPT=$(cat << 'EOF'
const { chromium } = require('playwright');

(async () => {
  let browser;
  try {
    browser = await chromium.launch({ 
      headless: true,
      timeout: 30000,
    });
    const page = await browser.newPage();
    await page.goto('about:blank');
    console.log('PLAYWRIGHT_OK');
  } catch (err) {
    console.error('PLAYWRIGHT_FAIL:', err.message);
    process.exit(1);
  } finally {
    if (browser) await browser.close();
  }
})();
EOF
)

if [[ "$DRY_RUN" == "false" ]]; then
  cd "$PROJECT_ROOT"
  
  PLAYWRIGHT_RESULT=$(node -e "$PLAYWRIGHT_TEST_SCRIPT" 2>&1 || true)
  
  if echo "$PLAYWRIGHT_RESULT" | grep -q "PLAYWRIGHT_OK"; then
    log_info "Playwright browser launch validated ✓"
  else
    log_warn "Playwright browser launch failed. Attempting repair..."
    
    # Try reinstalling with forced dependencies
    npx playwright install chromium --with-deps --force 2>&1 | tail -5
    
    # Retry validation
    PLAYWRIGHT_RETRY=$(node -e "$PLAYWRIGHT_TEST_SCRIPT" 2>&1 || true)
    
    if echo "$PLAYWRIGHT_RETRY" | grep -q "PLAYWRIGHT_OK"; then
      log_info "Playwright browser repaired and validated ✓"
    else
      log_error "Playwright browser still failing after repair. Continuing with degraded mode."
      log_error "Google Play scraping will be disabled. Only iTunes API will work."
    fi
  fi
else
  log_info "[DRY RUN] Would validate Playwright browser launch"
fi

# ============================================================================
# 4. PREPARE ONNX MODEL CACHE
# ============================================================================

log_info "Checking ONNX model cache directory..."

ONNX_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/huggingface/hub"

if [[ ! -d "$ONNX_CACHE" ]]; then
  log_info "Creating ONNX cache directory: $ONNX_CACHE"
  if [[ "$DRY_RUN" == "false" ]]; then
    mkdir -p "$ONNX_CACHE"
  fi
fi

log_info "ONNX cache directory ready at $ONNX_CACHE ✓"

# ============================================================================
# 5. CHECK DISK SPACE
# ============================================================================

log_info "Checking available disk space..."

# Require at least 2GB free for models and cache
REQUIRED_SPACE_MB=2048
AVAILABLE_SPACE_MB=$(df -m "$PROJECT_ROOT" | awk 'NR==2 {print $4}')

if [[ "$AVAILABLE_SPACE_MB" -lt "$REQUIRED_SPACE_MB" ]]; then
  log_warn "Low disk space: ${AVAILABLE_SPACE_MB}MB available, ${REQUIRED_SPACE_MB}MB recommended"
else
  log_info "Disk space OK: ${AVAILABLE_SPACE_MB}MB available ✓"
fi

# ============================================================================
# 6. BUILD PROJECT (if needed)
# ============================================================================

log_info "Checking for compiled TypeScript..."

if [[ ! -d "$PROJECT_ROOT/dist" ]] || [[ ! -f "$PROJECT_ROOT/dist/index.js" ]]; then
  log_warn "Compiled code not found. Building..."
  
  if [[ "$DRY_RUN" == "false" ]]; then
    cd "$PROJECT_ROOT"
    npm run build 2>&1 | tail -5
    log_info "Build completed ✓"
  else
    log_info "[DRY RUN] Would run: npm run build"
  fi
else
  log_info "Compiled TypeScript found in dist/ ✓"
fi

# ============================================================================
# 7. START WORKER
# ============================================================================

if [[ "$DRY_RUN" == "true" ]]; then
  log_info "[DRY RUN] All checks passed. Would start worker daemon."
  exit 0
fi

log_info "All startup checks passed. Starting worker daemon..."

cd "$PROJECT_ROOT"

# Export environment for clean startup
export NODE_ENV="${NODE_ENV:-production}"
export NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=1024}"

# Start the main process
exec node dist/index.js "$@"
