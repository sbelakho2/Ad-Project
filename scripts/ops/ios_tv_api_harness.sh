#!/usr/bin/env bash
# iOS & tvOS API Harness (Test Server Only)
# Generates real analytics events tied to the iOS and CTV (Apple TV) app IDs
# from the Test Server (Linux), since iOS/tvOS simulators cannot run here.
#
# Reads /opt/adproject/.env.slo for: API_URL, TOKEN, SDK_KEY, PUBLISHER_ID,
#   APP_ID_IOS, APP_ID_CTV
#
# Usage:
#   ./scripts/ops/ios_tv_api_harness.sh [duration_minutes] [events_per_minute]

set -euo pipefail

if [[ -f "/opt/adproject/.env.slo" ]]; then set -a; source "/opt/adproject/.env.slo"; set +a; fi
if [[ -f ".env.slo" ]]; then set -a; source ".env.slo"; set +a; fi

API_URL=${API_URL:-https://api.apexmediation.ee}
TOKEN=${TOKEN:-}
SDK_KEY=${SDK_KEY:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
APP_ID_IOS=${APP_ID_IOS:-}
APP_ID_CTV=${APP_ID_CTV:-}

DURATION_MINUTES=${1:-0}
EVENTS_PER_MINUTE=${2:-8}

require_env() { local n=$1 v=${2:-}; [[ -z "$v" ]] && { echo "[err] Missing env: $n" >&2; exit 2; }; }
require_env TOKEN "$TOKEN"
require_env SDK_KEY "$SDK_KEY"
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_env APP_ID_IOS "$APP_ID_IOS"
require_env APP_ID_CTV "$APP_ID_CTV"

EVID_DIR="evidence-$(date +%F)/ios_tvos"
LOG="$EVID_DIR/ios_tv_api_harness.log"
mkdir -p "$EVID_DIR"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

uuid() {
  printf '%04x%04x-%04x-%04x-%04x-%04x%04x%04x\n' \
    $RANDOM $RANDOM $RANDOM \
    $(($RANDOM & 0x0fff | 0x4000)) \
    $(($RANDOM & 0x3fff | 0x8000)) \
    $RANDOM $RANDOM $RANDOM
}

revenue() { echo "0.0$(printf '%02d' $((RANDOM % 80 + 20)))"; }

send_imp() {
  local app_id=$1 os=$2 device=$3 fmt=$4
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local events="[{\"event_id\":\"$(uuid)\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"adapter_name\":\"ios_tv_test\",\"ad_format\":\"$fmt\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$(uuid)\",\"request_id\":\"$(uuid)\",\"ecpm_usd\": $(revenue),\"timestamp\":\"$now\"}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/impressions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -H "X-SDK-Key: $SDK_KEY" \
    -d "{\"events\": $events}"
}

send_rev() {
  local app_id=$1 os=$2 device=$3 fmt=$4
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local events="[{\"event_id\":\"$(uuid)\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"adapter_name\":\"ios_tv_test\",\"revenue_type\":\"impression\",\"revenue_usd\": $(revenue),\"ad_format\":\"$fmt\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$(uuid)\",\"request_id\":\"$(uuid)\",\"timestamp\":\"$now\"}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/revenue" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -H "X-SDK-Key: $SDK_KEY" \
    -d "{\"events\": $events}"
}

SLEEP=$((60 / (EVENTS_PER_MINUTE>0?EVENTS_PER_MINUTE:1) ))
[[ $SLEEP -lt 1 ]] && SLEEP=1
log "iOS/tvOS API harness started (ios=$APP_ID_IOS, ctv=$APP_ID_CTV)"
START=$(date +%s)
ITER=0
while true; do
  ITER=$((ITER+1))
  code1=$(send_imp "$APP_ID_IOS" "ios" "phone" "interstitial")
  code2=$(send_imp "$APP_ID_CTV" "tvos" "tv" "video")
  if [[ $((ITER % 3)) -eq 0 ]]; then
    code3=$(send_rev "$APP_ID_IOS" "ios" "phone" "interstitial")
    code4=$(send_rev "$APP_ID_CTV" "tvos" "tv" "video")
  fi
  if [[ $((ITER % 20)) -eq 0 ]]; then
    log "iter=$ITER codes: ios_imp=$code1 tv_imp=$code2"
  fi
  if [[ $DURATION_MINUTES -gt 0 ]]; then
    ELAP=$(( ( $(date +%s) - START ) / 60 ))
    [[ $ELAP -ge $DURATION_MINUTES ]] && break
  fi
  sleep "$SLEEP"
done
log "iOS/tvOS API harness finished"
