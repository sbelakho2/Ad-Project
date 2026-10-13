#!/usr/bin/env bash
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
# ============================================================================
# DEPLOYMENT ARCHITECTURE VERIFICATION
# ============================================================================
# 
# Verifies that all services are deployed to the CORRECT servers:
#
#   APP SERVER (46.62.252.96):
#     ✓ control-plane.service  - Orchestrator, Admin API
#     ✓ backend (pm2)          - Main API
#     ✓ console (pm2)          - Admin Console
#     ✓ website (pm2)          - Public Website
#     ✓ edge-tunnel.service    - SSH tunnel to Edge Worker
#     ✗ edge-worker            - SHOULD NOT BE HERE!
#
#   TEST SERVER (157.90.126.205):
#     ✓ edge-worker.service    - ML inference (LLM, Classifier)
#     ✗ control-plane          - SHOULD NOT BE HERE!
#     ✗ backend/console        - SHOULD NOT BE HERE!
#
# Usage: ./scripts/ops/verify-deployment-architecture.sh
#
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SSH_SCRIPT="$SCRIPT_DIR/hetzner_ssh.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Expected IPs
APP_SERVER_IP="46.62.252.96"
TEST_SERVER_IP="157.90.126.205"

passed=0
failed=0

print_header() {
  echo ""
  echo -e "${CYAN}═══════════════════════════════════════════════════════════════${NC}"
  echo -e "${CYAN}  $1${NC}"
  echo -e "${CYAN}═══════════════════════════════════════════════════════════════${NC}"
}

check_service() {
  local server=$1
  local service=$2
  local expected=$3
  local result

  result=$("$SSH_SCRIPT" "$server" -- "systemctl is-active $service 2>/dev/null || echo 'not-found'" 2>&1 | tail -1)
  
  if [[ "$expected" == "running" ]]; then
    if [[ "$result" == "active" ]]; then
      echo -e "  ${GREEN}✓${NC} $service: ${GREEN}running${NC}"
      ((passed++))
    else
      echo -e "  ${RED}✗${NC} $service: ${RED}$result${NC} (expected: running)"
      ((failed++))
    fi
  else
    if [[ "$result" == "not-found" || "$result" == "inactive" ]]; then
      echo -e "  ${GREEN}✓${NC} $service: ${GREEN}not present${NC} (correct)"
      ((passed++))
    else
      echo -e "  ${RED}✗${NC} $service: ${RED}$result${NC} (should NOT be on this server!)"
      ((failed++))
    fi
  fi
}

check_pm2() {
  local server=$1
  local app=$2
  local expected=$3
  local result

  result=$("$SSH_SCRIPT" "$server" -- "pm2 show $app 2>/dev/null | grep -oP 'status.*\\K(online|stopped|errored)' || echo 'not-found'" 2>&1 | tail -1)
  
  if [[ "$expected" == "running" ]]; then
    if [[ "$result" == "online" ]]; then
      echo -e "  ${GREEN}✓${NC} $app (pm2): ${GREEN}online${NC}"
      ((passed++))
    else
      echo -e "  ${RED}✗${NC} $app (pm2): ${RED}$result${NC} (expected: online)"
      ((failed++))
    fi
  else
    if [[ "$result" == "not-found" ]]; then
      echo -e "  ${GREEN}✓${NC} $app (pm2): ${GREEN}not present${NC} (correct)"
      ((passed++))
    else
      echo -e "  ${YELLOW}!${NC} $app (pm2): ${YELLOW}$result${NC} (should not be here)"
      ((failed++))
    fi
  fi
}

check_connectivity() {
  local name=$1
  local url=$2
  local result

  result=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$url" 2>/dev/null || echo "000")
  
  if [[ "$result" =~ ^(200|302|401|403)$ ]]; then
    echo -e "  ${GREEN}✓${NC} $name: ${GREEN}HTTP $result${NC}"
    ((passed++))
  else
    echo -e "  ${RED}✗${NC} $name: ${RED}HTTP $result${NC}"
    ((failed++))
  fi
}

echo ""
echo -e "${BLUE}╔═══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       APEX MEDIATION DEPLOYMENT ARCHITECTURE VERIFIER         ║${NC}"
echo -e "${BLUE}╚═══════════════════════════════════════════════════════════════╝${NC}"

# ============================================================================
# APP SERVER CHECKS
# ============================================================================
print_header "APP SERVER ($APP_SERVER_IP)"

echo -e "\n  ${YELLOW}Expected services:${NC}"
check_service app control-plane running
check_service app edge-tunnel running
check_pm2 app backend running
check_pm2 app console running
check_pm2 app website running

echo -e "\n  ${YELLOW}Services that should NOT be here:${NC}"
check_service app edge-worker not-running

# ============================================================================
# TEST SERVER CHECKS
# ============================================================================
print_header "TEST SERVER ($TEST_SERVER_IP)"

echo -e "\n  ${YELLOW}Expected services:${NC}"
check_service test edge-worker running

echo -e "\n  ${YELLOW}Services that should NOT be here:${NC}"
check_service test control-plane not-running
check_pm2 test backend not-running
check_pm2 test console not-running

# ============================================================================
# CONNECTIVITY CHECKS
# ============================================================================
print_header "SERVICE CONNECTIVITY"

# Check Edge Worker via tunnel from App Server
echo -e "\n  ${YELLOW}Edge Worker (via SSH tunnel):${NC}"
tunnel_check=$("$SSH_SCRIPT" app -- 'curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:3500/health 2>/dev/null || echo "000"' 2>&1 | tail -1)
if [[ "$tunnel_check" == "200" ]]; then
  echo -e "  ${GREEN}✓${NC} Edge Worker via tunnel: ${GREEN}connected${NC}"
  ((passed++))
else
  echo -e "  ${RED}✗${NC} Edge Worker via tunnel: ${RED}HTTP $tunnel_check${NC}"
  ((failed++))
fi

# Check Control Plane health
echo -e "\n  ${YELLOW}Control Plane API:${NC}"
cp_health=$("$SSH_SCRIPT" app -- 'curl -s http://localhost:4000/api/autonomous/health 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get(\"edgeWorker\",{}).get(\"status\",\"unknown\"))" 2>/dev/null || echo "error"' 2>&1 | tail -1)
if [[ "$cp_health" == "connected" ]]; then
  echo -e "  ${GREEN}✓${NC} Control Plane → Edge Worker: ${GREEN}connected${NC}"
  ((passed++))
else
  echo -e "  ${RED}✗${NC} Control Plane → Edge Worker: ${RED}$cp_health${NC}"
  ((failed++))
fi

# ============================================================================
# SUMMARY
# ============================================================================
print_header "VERIFICATION SUMMARY"

total=$((passed + failed))
echo ""
if [[ $failed -eq 0 ]]; then
  echo -e "  ${GREEN}✓ All $passed checks passed!${NC}"
  echo ""
  echo -e "  ${GREEN}Architecture is correctly deployed:${NC}"
  echo -e "    • App Server (46.62.252.96): Control Plane + Apps"
  echo -e "    • Test Server (157.90.126.205): ML Edge Worker"
  echo ""
  exit 0
else
  echo -e "  ${GREEN}Passed:${NC} $passed"
  echo -e "  ${RED}Failed:${NC} $failed"
  echo ""
  echo -e "  ${RED}⚠️  Some checks failed! Review the output above.${NC}"
  echo ""
  exit 1
fi
