#!/usr/bin/env bash
# VRA Synthetic Load Test (Test Server)
# Generates schema-valid impressions + clicks + revenue, scoped to real apps via SDK key auth.
#
# Usage: ./scripts/ops/vra_synthetic_load.sh [duration_minutes] [events_per_minute]

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)

DURATION_MINUTES=${1:-60}
EVENTS_PER_MINUTE=${2:-20}

# Load optional env file for SLO runs if present
if [[ -f "/opt/adproject/.env.slo" ]]; then
  set -a; source "/opt/adproject/.env.slo"; set +a
elif [[ -f "$ROOT_DIR/.env.slo" ]]; then
  set -a; source "$ROOT_DIR/.env.slo"; set +a
fi

# Hetzner infrastructure (App: 46.62.252.96, DB: 77.42.44.35)
API_URL="${API_URL:-https://api.apexmediation.ee}"

# Optional (used only for VRA recon endpoints)
TOKEN="${TOKEN:-}"

# Default SDK key (optional if per-app SDK_KEY_* present)
SDK_KEY="${SDK_KEY:-}"

PUBLISHER_ID="${PUBLISHER_ID:-}"

# Per-SDK app IDs (must belong to the dummy publisher account)
APP_ID_ANDROID="${APP_ID_ANDROID:-}"
APP_ID_ANDROID_2="${APP_ID_ANDROID_2:-}"
APP_ID_IOS="${APP_ID_IOS:-}"
APP_ID_IOS_2="${APP_ID_IOS_2:-}"
APP_ID_WEB="${APP_ID_WEB:-}"
APP_ID_UNITY="${APP_ID_UNITY:-}"
APP_ID_CTV="${APP_ID_CTV:-}"

# Optional per-app SDK keys (preferred)
SDK_KEY_ANDROID="${SDK_KEY_ANDROID:-}"
SDK_KEY_ANDROID_2="${SDK_KEY_ANDROID_2:-}"
SDK_KEY_IOS="${SDK_KEY_IOS:-}"
SDK_KEY_IOS_2="${SDK_KEY_IOS_2:-}"
SDK_KEY_WEB="${SDK_KEY_WEB:-}"
SDK_KEY_UNITY="${SDK_KEY_UNITY:-}"
SDK_KEY_CTV="${SDK_KEY_CTV:-}"

EVIDENCE_DIR="$ROOT_DIR/evidence-$(date +%F)/vra"
LOG_FILE="$EVIDENCE_DIR/synthetic_load.log"
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

require_any_sdk_key() {
  if [[ -z "$SDK_KEY" && -z "$SDK_KEY_ANDROID" && -z "$SDK_KEY_IOS" && -z "$SDK_KEY_WEB" && -z "$SDK_KEY_UNITY" && -z "$SDK_KEY_CTV" ]]; then
    log "[err] Missing SDK key(s): set SDK_KEY or per-app SDK_KEY_*"
    exit 2
  fi
}

check_vra_status() {
  if [[ -z "$TOKEN" ]]; then
    echo "variance=N/A coverage=N/A total=0"
    return 0
  fi
  local overview
  overview=$(curl -s -H "Authorization: Bearer $TOKEN" "$API_URL/api/v1/recon/overview" 2>/dev/null || true)
  local variance
  variance=$(echo "$overview" | grep -o '"variancePercent":[0-9.]*' | cut -d: -f2 || echo "N/A")
  local coverage
  coverage=$(echo "$overview" | grep -o '"coveragePercent":[0-9.]*' | cut -d: -f2 || echo "N/A")
  local total
  total=$(echo "$overview" | grep -o '"totalReconciled":[0-9]*' | cut -d: -f2 || echo "0")
  echo "variance=${variance}% coverage=${coverage}% total=$total"
}

post_batches() {
  local kind="$1"  # impressions|clicks|revenue
  local json_file="$2"

  local endpoint="$API_URL/api/v1/analytics/events/$kind"
  [[ "$kind" == "revenue" ]] && endpoint="$API_URL/api/v1/analytics/events/revenue"

  python3 - "$json_file" "$kind" <<'PY' | while IFS=$'\t' read -r sdk_key events_json; do
import json
import sys

data = json.load(open(sys.argv[1], 'r'))
kind = sys.argv[2]
for b in data.get(kind, []):
  k = b.get('sdk_key', '')
  ev = b.get('events', [])
  if not k or not ev:
    continue
  print(k + '\t' + json.dumps(ev, separators=(',', ':')))
PY
    local resp
    resp=$(curl -s -w "\n%{http_code}" -X POST "$endpoint" --config - \
      -d "{\"events\": $events_json}" 2>/dev/null <<EOF
header = "Content-Type: application/json"
header = "X-SDK-Key: $sdk_key"
EOF
    )
    local code
    code=$(echo "$resp" | tail -n1)
    if [[ "$code" != "202" ]]; then
      sdk_key_suffix=$(printf "%s" "$sdk_key" | tail -c 7)
      log "[warn] $kind HTTP $code (sdk_key=...$sdk_key_suffix)"
      echo "$resp" | head -n 5 >> "$LOG_FILE"
    fi
  done
}

send_events_once() {
  local batch_size="$1"
  local generator="$ROOT_DIR/tools/slo_event_generator.py"
  if [[ ! -f "$generator" ]]; then
    log "[err] Missing generator: $generator"
    return 1
  fi

  local ts
  ts=$(date +%s)
  local out="$EVIDENCE_DIR/events_${ts}.json"
  python3 "$generator" --count "$batch_size" > "$out"

  post_batches impressions "$out"
  post_batches clicks "$out"
  post_batches revenue "$out"

  python3 - "$out" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], 'r'))

def count(kind: str) -> int:
    return sum(len(b.get('events', [])) for b in data.get(kind, []))

print(
    "impressions=%d clicks=%d revenue=%d"
    % (count('impressions'), count('clicks'), count('revenue'))
)
PY
}

# Main
log "=========================================="
log "VRA Synthetic Load Test"
log "Duration: ${DURATION_MINUTES} minutes"
log "Target: ~${EVENTS_PER_MINUTE} events/minute"
log "API: $API_URL"
log "=========================================="

require_any_sdk_key
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_env APP_ID_ANDROID "$APP_ID_ANDROID"
require_env APP_ID_IOS "$APP_ID_IOS"
require_env APP_ID_WEB "$APP_ID_WEB"

log "Checking API health..."
HEALTH=$(curl -s "$API_URL/health" 2>/dev/null || echo '{"error":"unreachable"}')
log "Health: $HEALTH"
if echo "$HEALTH" | grep -q "error"; then
  log "❌ API not reachable. Exiting."
  exit 1
fi

log "Initial VRA status: $(check_vra_status)"

END_TIME=$(($(date +%s) + DURATION_MINUTES * 60))
ITERATION=0
TOTAL_IMPRESSIONS=0
TOTAL_CLICKS=0
TOTAL_REVENUE=0

SLEEP_INTERVAL=$((60 / EVENTS_PER_MINUTE))
[[ $SLEEP_INTERVAL -lt 1 ]] && SLEEP_INTERVAL=1

while [[ $(date +%s) -lt $END_TIME ]]; do
  ITERATION=$((ITERATION + 1))

  local_summary=$(send_events_once 25 || echo "impressions=0 clicks=0 revenue=0")
  imp_count=$(echo "$local_summary" | sed -n 's/.*impressions=\([0-9]\+\).*/\1/p')
  clk_count=$(echo "$local_summary" | sed -n 's/.*clicks=\([0-9]\+\).*/\1/p')
  rev_count=$(echo "$local_summary" | sed -n 's/.*revenue=\([0-9]\+\).*/\1/p')
  [[ -n "$imp_count" ]] && TOTAL_IMPRESSIONS=$((TOTAL_IMPRESSIONS + imp_count))
  [[ -n "$clk_count" ]] && TOTAL_CLICKS=$((TOTAL_CLICKS + clk_count))
  [[ -n "$rev_count" ]] && TOTAL_REVENUE=$((TOTAL_REVENUE + rev_count))

  if [[ $((ITERATION % 10)) -eq 0 ]]; then
    REMAINING=$((END_TIME - $(date +%s)))
    log "Progress: iter=$ITERATION imp=$TOTAL_IMPRESSIONS clk=$TOTAL_CLICKS rev=$TOTAL_REVENUE remaining=${REMAINING}s VRA=$(check_vra_status)"
  fi

  sleep "$SLEEP_INTERVAL"
done

log "=========================================="
log "Load test complete"
log "Total impressions sent: $TOTAL_IMPRESSIONS"
log "Total clicks sent: $TOTAL_CLICKS"
log "Total revenue events sent: $TOTAL_REVENUE"
log "=========================================="

log "Waiting 30s for VRA to process..."
sleep 30

log "Final VRA status: $(check_vra_status)"

if [[ -n "$TOKEN" ]]; then
  DELTAS=$(curl -s -H "Authorization: Bearer $TOKEN" "$API_URL/api/v1/recon/deltas" 2>/dev/null || true)
  DELTA_COUNT=$(echo "$DELTAS" | grep -o '"total":[0-9]*' | cut -d: -f2 || echo "0")
  log "Delta count: $DELTA_COUNT"

  if [[ "$DELTA_COUNT" != "0" && "$DELTA_COUNT" != "" ]]; then
    log "⚠️ Discrepancies detected - saving to evidence"
    echo "$DELTAS" > "$EVIDENCE_DIR/deltas_$(date +%Y%m%d_%H%M%S).json"
  else
    log "✅ No discrepancies detected"
  fi
else
  log "[info] TOKEN not set; skipped VRA delta checks"
fi

log "Evidence saved to: $EVIDENCE_DIR"
log "Test complete"
