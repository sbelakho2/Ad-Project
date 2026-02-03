#!/usr/bin/env bash
# Deploy mail server to test server
# Usage: ./deploy.sh [build|deploy|test-email]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(cd "$PROJECT_DIR/../.." && pwd)"

# Server config
TEST_SERVER="157.90.126.205"
SSH_KEY="${APEX_HETZNER_SSH_KEY:-$HOME/.ssh/hetzner-db-mac}"
REMOTE_PATH="/opt/apex-mail"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

ssh_cmd() {
  ssh -i "$SSH_KEY" -o BatchMode=yes -o ConnectTimeout=10 "root@${TEST_SERVER}" "$@"
}

cmd_build() {
  log_info "Building mail server (local)..."
  cd "$PROJECT_DIR"
  
  # Check for Rust
  if ! command -v cargo &>/dev/null; then
    log_error "Rust not installed. Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
  fi
  
  cargo build --release
  log_info "Build complete"
}

cmd_deploy() {
  log_info "Deploying mail server to test server..."
  
  # Create remote directory
  ssh_cmd "mkdir -p ${REMOTE_PATH}/bin ${REMOTE_PATH}/config ${REMOTE_PATH}/scripts"
  
  # Copy scripts
  log_info "Copying scripts..."
  rsync -avz -e "ssh -i $SSH_KEY" \
    "$PROJECT_DIR/scripts/" \
    "root@${TEST_SERVER}:${REMOTE_PATH}/scripts/"
  
  # Copy config template
  log_info "Copying config..."
  rsync -avz -e "ssh -i $SSH_KEY" \
    "$PROJECT_DIR/config/" \
    "root@${TEST_SERVER}:${REMOTE_PATH}/config/" 2>/dev/null || true
  
  # Copy binaries if built
  if [[ -d "$PROJECT_DIR/target/release" ]]; then
    log_info "Copying binaries..."
    for bin in mailstore-core smtp-edge submission imap-edge outbound-queue spam-gateway send-test-email; do
      if [[ -f "$PROJECT_DIR/target/release/$bin" ]]; then
        rsync -avz -e "ssh -i $SSH_KEY" \
          "$PROJECT_DIR/target/release/$bin" \
          "root@${TEST_SERVER}:${REMOTE_PATH}/bin/"
      fi
    done
  fi
  
  # Make scripts executable
  ssh_cmd "chmod +x ${REMOTE_PATH}/scripts/*.sh"
  
  # Setup env from main project
  log_info "Setting up environment..."
  ssh_cmd "grep RESEND_API_KEY /opt/adproject/.env > ${REMOTE_PATH}/.env 2>/dev/null || true"
  
  log_info "Deployment complete"
}

cmd_test_email() {
  local TO_EMAIL="${1:-}"
  if [[ -z "$TO_EMAIL" ]]; then
    log_error "Usage: $0 test-email <to_email>"
    exit 1
  fi
  
  log_info "Sending test email to ${TO_EMAIL}..."
  
  ssh_cmd "cd ${REMOTE_PATH} && source /opt/adproject/.env && bash scripts/send-test-email.sh '${TO_EMAIL}'"
}

cmd_setup_rust() {
  log_info "Installing Rust on test server..."
  
  ssh_cmd "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
  ssh_cmd "source ~/.cargo/env && rustc --version"
  
  log_info "Rust installed on test server"
}

cmd_status() {
  log_info "Checking mail server status on test server..."
  
  ssh_cmd "
    echo '=== Mail Server Status ==='
    echo ''
    echo 'Directory:'
    ls -la ${REMOTE_PATH}/ 2>/dev/null || echo 'Not deployed'
    echo ''
    echo 'Binaries:'
    ls -la ${REMOTE_PATH}/bin/ 2>/dev/null || echo 'No binaries'
    echo ''
    echo 'Scripts:'
    ls -la ${REMOTE_PATH}/scripts/ 2>/dev/null || echo 'No scripts'
    echo ''
    echo 'Environment:'
    cat ${REMOTE_PATH}/.env 2>/dev/null | sed 's/=.*/=***/' || echo 'No env'
    echo ''
    echo 'Services:'
    systemctl is-active mailstore-core 2>/dev/null || echo 'mailstore-core: not installed'
    systemctl is-active smtp-edge 2>/dev/null || echo 'smtp-edge: not installed'
    systemctl is-active outbound-queue 2>/dev/null || echo 'outbound-queue: not installed'
  "
}

cmd_help() {
  cat <<EOF
ApexMediation Mail Server Deployment

Usage: $0 <command> [args]

Commands:
  build         Build mail server locally
  deploy        Deploy to test server
  test-email    Send test email via Resend
  setup-rust    Install Rust on test server
  status        Check mail server status

Examples:
  $0 build
  $0 deploy
  $0 test-email sadokaaronbelakhoua@gmail.com
  $0 status
EOF
}

# Main
case "${1:-help}" in
  build) cmd_build ;;
  deploy) cmd_deploy ;;
  test-email) cmd_test_email "${2:-}" ;;
  setup-rust) cmd_setup_rust ;;
  status) cmd_status ;;
  help|--help|-h) cmd_help ;;
  *) 
    log_error "Unknown command: $1"
    cmd_help
    exit 1
    ;;
esac
