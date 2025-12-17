#!/usr/bin/env bash
# Web SDK Harness (Test Server Only)
# Continuously generates production-like analytics activity for the Web app ID
# tied to the dummy publisher, using real ingestion endpoints.
#
# Reads /opt/adproject/.env.slo if present for:
#   API_URL, TOKEN, SDK_KEY, PUBLISHER_ID, APP_ID_WEB
#
# Usage:
#   ./scripts/ops/web_sdk_harness.sh [duration_minutes] [events_per_minute]

set -euo pipefail

# Load env
if [[ -f "/opt/adproject/.env.slo" ]]; then set -a; source "/opt/adproject/.env.slo"; set +a; fi
if [[ -f ".env.slo" ]]; then set -a; source ".env.slo"; set +a; fi

API_URL=${API_URL:-https://api.apexmediation.ee}
TOKEN=${TOKEN:-}
SDK_KEY=${SDK_KEY:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
APP_ID_WEB=${APP_ID_WEB:-}

DURATION_MINUTES=${1:-0}   # 0 = run indefinitely
EVENTS_PER_MINUTE=${2:-12}

require_env() { local n=$1 v=${2:-}; [[ -z "$v" ]] && { echo "[err] Missing env: $n" >&2; exit 2; }; }
require_env TOKEN "$TOKEN"
require_env SDK_KEY "$SDK_KEY"
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_env APP_ID_WEB "$APP_ID_WEB"

EVID_DIR="evidence-$(date +%F)/web"
LOG="$EVID_DIR/web_sdk_harness.log"
mkdir -p "$EVID_DIR"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

random_uuid() {
  printf '%04x%04x-%04x-%04x-%04x-%04x%04x%04x\n' \
    $RANDOM $RANDOM $RANDOM \
    $(($RANDOM & 0x0fff | 0x4000)) \
    $(($RANDOM & 0x3fff | 0x8000)) \
    $RANDOM $RANDOM $RANDOM
}

random_revenue() { echo "0.0$(printf '%02d' $((RANDOM % 90 + 10)))"; }

send_web_impressions() {
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local events="[{\"event_id\":\"$(random_uuid)\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$APP_ID_WEB\",\"adapter_name\":\"web_test\",\"ad_format\":\"banner\",\"country_code\":\"US\",\"device_type\":\"desktop\",\"os\":\"web\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$(random_uuid)\",\"request_id\":\"$(random_uuid)\",\"ecpm_usd\": $(random_revenue),\"timestamp\":\"$now\"}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/impressions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -H "X-SDK-Key: $SDK_KEY" \
    -d "{\"events\": $events}"
}

send_web_revenue() {
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local events="[{\"event_id\":\"$(random_uuid)\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$APP_ID_WEB\",\"adapter_name\":\"web_test\",\"revenue_type\":\"impression\",\"revenue_usd\": $(random_revenue),\"ad_format\":\"banner\",\"country_code\":\"US\",\"device_type\":\"desktop\",\"os\":\"web\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$(random_uuid)\",\"request_id\":\"$(random_uuid)\",\"timestamp\":\"$now\"}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/revenue" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -H "X-SDK-Key: $SDK_KEY" \
    -d "{\"events\": $events}"
}

SLEEP_INTERVAL=$((60 / (EVENTS_PER_MINUTE>0?EVENTS_PER_MINUTE:1) ))
[[ $SLEEP_INTERVAL -lt 1 ]] && SLEEP_INTERVAL=1

log "Web SDK Harness started: $EVENTS_PER_MINUTE epm (APP_ID_WEB=$APP_ID_WEB)"
START=$(date +%s)
ITER=0
while true; do
  ITER=$((ITER+1))
  code1=$(send_web_impressions)
  [[ $((ITER % 3)) -eq 0 ]] && code2=$(send_web_revenue) || code2="-"
  if [[ $((ITER % 20)) -eq 0 ]]; then
    log "iter=$ITER codes: imp=$code1 rev=$code2"
  fi
  if [[ $DURATION_MINUTES -gt 0 ]]; then
    ELAP=$(( ( $(date +%s) - START ) / 60 ))
    [[ $ELAP -ge $DURATION_MINUTES ]] && break
  fi
  sleep "$SLEEP_INTERVAL"
done
log "Web SDK Harness finished"
