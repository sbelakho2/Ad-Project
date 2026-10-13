#!/usr/bin/env bash
# VRA Multi-Day Soak Test
# Runs continuous VRA reconciliation checks over multiple days
# Usage: ./scripts/ops/vra_multiday_test.sh

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

# Load optional env file for SLO runs if present
if [[ -f "/opt/adproject/.env.slo" ]]; then
  set -a; source "/opt/adproject/.env.slo"; set +a
elif [[ -f ".env.slo" ]]; then
  set -a; source ".env.slo"; set +a
fi

# Hetzner infrastructure
APP_SERVER="46.62.252.96"
DB_SERVER="77.42.44.35"
SSH_KEY="$HOME/.ssh/hetzner-db-mac"
SSH_CMD="ssh -i $SSH_KEY root@$DB_SERVER"
API_URL="${API_URL:-https://api.apexmediation.ee}"
TOKEN="${TOKEN:-}"
SDK_KEY="${SDK_KEY:-}"
EVIDENCE_DIR="evidence-$(date +%F)/vra"
LOG_FILE="$EVIDENCE_DIR/multiday_test.log"
INTERVAL_SECONDS=300  # 5 minutes between checks

mkdir -p "$EVIDENCE_DIR"

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

require_env() {
  local name="$1"; local value="${2:-}"
  if [[ -z "$value" ]]; then
    log "[err] Missing required env: $name"
    exit 2
  fi
}

check_vra_endpoints() {
  log "=== VRA Health Check ==="
  
  # 1. Recon Overview
  log "Checking /api/v1/recon/overview..."
  OVERVIEW=$(curl -s "$API_URL/api/v1/recon/overview" -H "Authorization: Bearer $TOKEN")
  log "Overview: $OVERVIEW"
  
  # 2. Recon Deltas (last 7 days)
  log "Checking /api/v1/recon/deltas..."
  FROM_DATE=$(date -u -v-7d +%Y-%m-%dT00:00:00Z 2>/dev/null || date -u -d '7 days ago' +%Y-%m-%dT00:00:00Z)
  TO_DATE=$(date -u +%Y-%m-%dT23:59:59Z)
  DELTAS=$(curl -s "$API_URL/api/v1/recon/deltas?from=$FROM_DATE&to=$TO_DATE" -H "Authorization: Bearer $TOKEN")
  DELTA_COUNT=$(echo "$DELTAS" | jq '.data | length' 2>/dev/null || echo "0")
  log "Delta count (last 7d): $DELTA_COUNT"
  
  # 3. Monthly Revenue Digest (proof-of-revenue)
  log "Checking /api/v1/proofs/revenue_digest..."
  CURRENT_MONTH=$(date +%Y-%m)
  DIGEST=$(curl -s "$API_URL/api/v1/proofs/revenue_digest?month=$CURRENT_MONTH" -H "Authorization: Bearer $TOKEN")
  log "Digest: $DIGEST"
  
  # 4. Database health via SSH
  log "Checking database tables..."
  RECON_COUNTS=$($SSH_CMD "docker exec ad-project-postgres-1 psql -U postgres -d apexmediation -t -c \"
    SELECT 'recon_expected' as tbl, COUNT(*) as cnt FROM recon_expected UNION ALL
    SELECT 'recon_statements_norm', COUNT(*) FROM recon_statements_norm UNION ALL
    SELECT 'recon_deltas', COUNT(*) FROM recon_deltas UNION ALL
    SELECT 'recon_match', COUNT(*) FROM recon_match;
  \"" 2>/dev/null || echo "DB query failed")
  log "Table counts:"
  log "$RECON_COUNTS"
  
  return 0
}

generate_synthetic_activity() {
  log "=== Generating Synthetic VRA Activity ==="
  
  # Generate a few RTB bid requests to create revenue events
  for i in $(seq 1 5); do
    RESP=$(curl -s -X POST "$API_URL/api/v1/rtb/bid" \
      -H "Content-Type: application/json" \
      -H "X-SDK-Key: $SDK_KEY" \
      -d '{
        "placementId": "test-placement-1",
        "adType": "banner",
        "width": 320,
        "height": 50,
        "deviceType": "phone",
        "os": "iOS",
        "osVersion": "17.0",
        "appVersion": "1.0.0"
      }' 2>/dev/null || echo '{"error":"request failed"}')
    log "RTB bid $i: $(echo "$RESP" | jq -c '.success // .error' 2>/dev/null || echo "$RESP")"
  done
  
  log "Synthetic activity generated"
}

run_reconcile_check() {
  log "=== Running Reconcile Check ==="
  
  # Check for variance/discrepancies
  OVERVIEW=$(curl -s "$API_URL/api/v1/recon/overview" -H "Authorization: Bearer $TOKEN")
  VARIANCE=$(echo "$OVERVIEW" | jq '.data.variancePercent' 2>/dev/null || echo "null")
  COVERAGE=$(echo "$OVERVIEW" | jq '.data.coveragePercent' 2>/dev/null || echo "null")
  
  log "Variance: ${VARIANCE}%"
  log "Coverage: ${COVERAGE}%"
  
  if [[ "$VARIANCE" != "null" && "$VARIANCE" != "0" ]]; then
    log "⚠️ Non-zero variance detected: ${VARIANCE}%"
  else
    log "✅ No variance detected"
  fi
}

# Main loop
log "=========================================="
log "Starting VRA Multi-Day Soak Test"
log "Interval: ${INTERVAL_SECONDS}s"
log "Evidence dir: $EVIDENCE_DIR"
log "=========================================="

# Validate env
require_env TOKEN "$TOKEN"
require_env SDK_KEY "$SDK_KEY"

ITERATION=0
while true; do
  ITERATION=$((ITERATION + 1))
  log ""
  log "=========================================="
  log "Iteration $ITERATION - $(date)"
  log "=========================================="
  
  # Health check
  if ! check_vra_endpoints; then
    log "❌ VRA health check failed"
  fi
  
  # Generate some activity every few iterations
  if [[ $((ITERATION % 3)) -eq 1 ]]; then
    generate_synthetic_activity
  fi
  
  # Run reconcile check
  run_reconcile_check
  
  log "Next check in ${INTERVAL_SECONDS}s..."
  log ""
  
  sleep "$INTERVAL_SECONDS"
done
