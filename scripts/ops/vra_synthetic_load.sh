#!/usr/bin/env bash
# VRA Synthetic Load Test
# Generates synthetic impressions, clicks, and revenue events to test VRA reconciliation
# Usage: ./scripts/ops/vra_synthetic_load.sh [duration_minutes] [events_per_minute]

set -euo pipefail

DURATION_MINUTES=${1:-60}
EVENTS_PER_MINUTE=${2:-20}
API_URL="http://164.90.232.151:8080"
TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiJlYjhiNjZkNC1mYTIxLTQ2NDQtOTVmMC1jNjU3YjNjZGY4OGYiLCJwdWJsaXNoZXJJZCI6ImVmYmIwMzk0LTIzZWItNGI4MC1hNGQ2LTlhMGU2MGVmMWI1NiIsImVtYWlsIjoidGVzdEBhcGV4bWVkaWF0aW9uLmVlIiwicm9sZSI6ImFkbWluIiwidG9rZW5UeXBlIjoiYWNjZXNzIiwiaWF0IjoxNzY1NTQzNzk4LCJleHAiOjE3NjYxNDg1OTh9.2Mk2Ja8o9ofzUI6hMbKwx0HQ85eXymqv4SJjv7xblg4"
SDK_KEY="ak_test_38dc38e6-7b2e-4f8a-9c5d-1e2f3a4b5c6d"
EVIDENCE_DIR="evidence-$(date +%F)/vra"
LOG_FILE="$EVIDENCE_DIR/synthetic_load.log"

mkdir -p "$EVIDENCE_DIR"

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

# Networks to simulate
NETWORKS=("admob" "unity" "applovin" "ironsource" "vungle")
PLACEMENTS=("banner_home" "interstitial_levelup" "rewarded_bonus" "native_feed" "mrec_sidebar")
DEVICES=("phone" "tablet")
OS_LIST=("ios" "android")

random_element() {
  local arr=("$@")
  echo "${arr[$RANDOM % ${#arr[@]}]}"
}

random_uuid() {
  # Generate a valid UUID v4
  printf '%04x%04x-%04x-%04x-%04x-%04x%04x%04x\n' \
    $RANDOM $RANDOM $RANDOM \
    $(($RANDOM & 0x0fff | 0x4000)) \
    $(($RANDOM & 0x3fff | 0x8000)) \
    $RANDOM $RANDOM $RANDOM
}

random_revenue() {
  # Random revenue between $0.001 and $0.10
  echo "0.0$(printf '%02d' $((RANDOM % 100 + 1)))"
}

random_latency() {
  # Random latency between 50 and 500ms
  echo "$((RANDOM % 450 + 50))"
}

generate_impression_batch() {
  local batch_size=$1
  local events="["
  
  for ((i=0; i<batch_size; i++)); do
    local network=$(random_element "${NETWORKS[@]}")
    local placement=$(random_element "${PLACEMENTS[@]}")
    local device=$(random_element "${DEVICES[@]}")
    local os=$(random_element "${OS_LIST[@]}")
    local event_id=$(random_uuid)
    local app_id=$(random_uuid)
    local placement_uuid=$(random_uuid)
    local adapter_uuid=$(random_uuid)
    local session_id=$(random_uuid)
    local request_id=$(random_uuid)
    local user_id="user_${RANDOM}"
    local timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    local ecpm=$(random_revenue)
    local bid=$(random_revenue)
    local latency=$(random_latency)
    
    if [[ $i -gt 0 ]]; then events+=","; fi
    events+="{
      \"event_id\": \"$event_id\",
      \"publisher_id\": \"efbb0394-23eb-4b80-a4d6-9a0e60ef1b56\",
      \"app_id\": \"$app_id\",
      \"placement_id\": \"$placement_uuid\",
      \"adapter_id\": \"$adapter_uuid\",
      \"adapter_name\": \"$network\",
      \"ad_unit_id\": \"ca-app-pub-test/$network\",
      \"ad_format\": \"banner\",
      \"country_code\": \"US\",
      \"device_type\": \"$device\",
      \"os\": \"$os\",
      \"os_version\": \"17.0\",
      \"app_version\": \"1.0.0\",
      \"sdk_version\": \"1.0.0\",
      \"session_id\": \"$session_id\",
      \"user_id\": \"$user_id\",
      \"request_id\": \"$request_id\",
      \"bid_price_usd\": $bid,
      \"ecpm_usd\": $ecpm,
      \"latency_ms\": $latency,
      \"is_test_mode\": true,
      \"timestamp\": \"$timestamp\"
    }"
  done
  
  events+="]"
  echo "$events"
}

generate_revenue_batch() {
  local batch_size=$1
  local events="["
  
  for ((i=0; i<batch_size; i++)); do
    local network=$(random_element "${NETWORKS[@]}")
    local placement=$(random_element "${PLACEMENTS[@]}")
    local device=$(random_element "${DEVICES[@]}")
    local os=$(random_element "${OS_LIST[@]}")
    local revenue=$(random_revenue)
    local event_id=$(random_uuid)
    local app_id=$(random_uuid)
    local placement_uuid=$(random_uuid)
    local adapter_uuid=$(random_uuid)
    local session_id=$(random_uuid)
    local request_id=$(random_uuid)
    local user_id="user_${RANDOM}"
    local timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    
    if [[ $i -gt 0 ]]; then events+=","; fi
    events+="{
      \"event_id\": \"$event_id\",
      \"publisher_id\": \"efbb0394-23eb-4b80-a4d6-9a0e60ef1b56\",
      \"app_id\": \"$app_id\",
      \"placement_id\": \"$placement_uuid\",
      \"adapter_id\": \"$adapter_uuid\",
      \"adapter_name\": \"$network\",
      \"ad_unit_id\": \"ca-app-pub-test/$network\",
      \"ad_format\": \"banner\",
      \"country_code\": \"US\",
      \"device_type\": \"$device\",
      \"os\": \"$os\",
      \"os_version\": \"17.0\",
      \"app_version\": \"1.0.0\",
      \"sdk_version\": \"1.0.0\",
      \"session_id\": \"$session_id\",
      \"user_id\": \"$user_id\",
      \"request_id\": \"$request_id\",
      \"revenue_usd\": $revenue,
      \"revenue_type\": \"impression\",
      \"is_test_mode\": true,
      \"timestamp\": \"$timestamp\"
    }"
  done
  
  events+="]"
  echo "$events"
}

send_impressions() {
  local batch=$(generate_impression_batch 5)
  local resp=$(curl -s -w "\n%{http_code}" -X POST "$API_URL/api/v1/analytics/events/impressions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -H "X-SDK-Key: $SDK_KEY" \
    -d "{\"events\": $batch}" 2>/dev/null)
  
  local status_code=$(echo "$resp" | tail -n1)
  local body=$(echo "$resp" | sed '$d')
  
  if [[ "$status_code" == "200" || "$status_code" == "201" || "$status_code" == "202" ]]; then
    echo "✓"
  else
    echo "✗($status_code)"
  fi
}

send_revenue() {
  local batch=$(generate_revenue_batch 5)
  local resp=$(curl -s -w "\n%{http_code}" -X POST "$API_URL/api/v1/analytics/events/revenue" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -H "X-SDK-Key: $SDK_KEY" \
    -d "{\"events\": $batch}" 2>/dev/null)
  
  local status_code=$(echo "$resp" | tail -n1)
  local body=$(echo "$resp" | sed '$d')
  
  if [[ "$status_code" == "200" || "$status_code" == "201" || "$status_code" == "202" ]]; then
    echo "✓"
  else
    echo "✗($status_code)"
  fi
}

check_vra_status() {
  local overview=$(curl -s "$API_URL/api/v1/recon/overview" -H "Authorization: Bearer $TOKEN" 2>/dev/null)
  local variance=$(echo "$overview" | grep -o '"variancePercent":[0-9.]*' | cut -d: -f2 || echo "N/A")
  local coverage=$(echo "$overview" | grep -o '"coveragePercent":[0-9.]*' | cut -d: -f2 || echo "N/A")
  local total=$(echo "$overview" | grep -o '"totalReconciled":[0-9]*' | cut -d: -f2 || echo "0")
  echo "variance=${variance}% coverage=${coverage}% total=$total"
}

# Main
log "=========================================="
log "VRA Synthetic Load Test"
log "Duration: ${DURATION_MINUTES} minutes"
log "Target: ~${EVENTS_PER_MINUTE} events/minute"
log "API: $API_URL"
log "=========================================="

# Initial health check
log "Checking API health..."
HEALTH=$(curl -s "$API_URL/health" 2>/dev/null || echo '{"error":"unreachable"}')
log "Health: $HEALTH"

if echo "$HEALTH" | grep -q "error"; then
  log "❌ API not reachable. Exiting."
  exit 1
fi

# Initial VRA status
log "Initial VRA status: $(check_vra_status)"

END_TIME=$(($(date +%s) + DURATION_MINUTES * 60))
ITERATION=0
TOTAL_IMPRESSIONS=0
TOTAL_REVENUE=0
SLEEP_INTERVAL=$((60 / EVENTS_PER_MINUTE))
[[ $SLEEP_INTERVAL -lt 1 ]] && SLEEP_INTERVAL=1

while [[ $(date +%s) -lt $END_TIME ]]; do
  ITERATION=$((ITERATION + 1))
  
  # Send impressions
  IMP_RESULT=$(send_impressions)
  if [[ "$IMP_RESULT" == "✓" ]]; then
    TOTAL_IMPRESSIONS=$((TOTAL_IMPRESSIONS + 5))
  fi
  
  # Send revenue (every other iteration)
  if [[ $((ITERATION % 2)) -eq 0 ]]; then
    REV_RESULT=$(send_revenue)
    if [[ "$REV_RESULT" == "✓" ]]; then
      TOTAL_REVENUE=$((TOTAL_REVENUE + 5))
    fi
  fi
  
  # Log progress every 10 iterations
  if [[ $((ITERATION % 10)) -eq 0 ]]; then
    ELAPSED=$(($(date +%s) - END_TIME + DURATION_MINUTES * 60))
    REMAINING=$((END_TIME - $(date +%s)))
    log "Progress: iter=$ITERATION imp=$TOTAL_IMPRESSIONS rev=$TOTAL_REVENUE remaining=${REMAINING}s VRA=$(check_vra_status)"
  fi
  
  sleep $SLEEP_INTERVAL
done

log "=========================================="
log "Load test complete"
log "Total impressions sent: $TOTAL_IMPRESSIONS"
log "Total revenue events sent: $TOTAL_REVENUE"
log "=========================================="

# Final VRA check
log "Waiting 30s for VRA to process..."
sleep 30

log "Final VRA status: $(check_vra_status)"

# Check for deltas (discrepancies)
DELTAS=$(curl -s "$API_URL/api/v1/recon/deltas" -H "Authorization: Bearer $TOKEN" 2>/dev/null)
DELTA_COUNT=$(echo "$DELTAS" | grep -o '"total":[0-9]*' | cut -d: -f2 || echo "0")
log "Delta count: $DELTA_COUNT"

if [[ "$DELTA_COUNT" != "0" && "$DELTA_COUNT" != "" ]]; then
  log "⚠️ Discrepancies detected - saving to evidence"
  echo "$DELTAS" > "$EVIDENCE_DIR/deltas_$(date +%Y%m%d_%H%M%S).json"
else
  log "✅ No discrepancies detected"
fi

log "Evidence saved to: $EVIDENCE_DIR"
log "Test complete"
