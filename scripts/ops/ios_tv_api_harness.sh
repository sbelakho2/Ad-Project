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
SDK_KEY_IOS=${SDK_KEY_IOS:-}
SDK_KEY_CTV=${SDK_KEY_CTV:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
APP_ID_IOS=${APP_ID_IOS:-}
APP_ID_CTV=${APP_ID_CTV:-}

DURATION_MINUTES=${1:-0}
EVENTS_PER_MINUTE=${2:-8}

require_env() {
  local name=$1 value=${2:-}
  if [[ -z "$value" ]]; then
    echo "[err] Missing env: $name" >&2
    exit 2
  fi
}
SDK_KEY_IOS_EFFECTIVE=${SDK_KEY_IOS:-$SDK_KEY}
SDK_KEY_CTV_EFFECTIVE=${SDK_KEY_CTV:-$SDK_KEY}
require_env SDK_KEY_IOS "$SDK_KEY_IOS_EFFECTIVE"
require_env SDK_KEY_CTV "$SDK_KEY_CTV_EFFECTIVE"
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_env APP_ID_IOS "$APP_ID_IOS"
require_env APP_ID_CTV "$APP_ID_CTV"

# Avoid leaking secrets via process args (ps/systemctl). Use a curl config file
# for headers instead of passing -H "Authorization: Bearer ..." on the command line.
CURL_CFG_IOS="$(mktemp)"
CURL_CFG_CTV="$(mktemp)"
chmod 600 "$CURL_CFG_IOS" "$CURL_CFG_CTV" || true
cat >"$CURL_CFG_IOS" <<EOF
header = "Content-Type: application/json"
header = "X-SDK-Key: $SDK_KEY_IOS_EFFECTIVE"
EOF
cat >"$CURL_CFG_CTV" <<EOF
header = "Content-Type: application/json"
header = "X-SDK-Key: $SDK_KEY_CTV_EFFECTIVE"
EOF
if [[ -n "$TOKEN" ]]; then
  echo "header = \"Authorization: Bearer $TOKEN\"" >>"$CURL_CFG_IOS"
  echo "header = \"Authorization: Bearer $TOKEN\"" >>"$CURL_CFG_CTV"
fi
cleanup() { rm -f "$CURL_CFG_IOS" "$CURL_CFG_CTV" 2>/dev/null || true; }
trap cleanup EXIT

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
latency() { echo "$((RANDOM % 200 + 50))"; }

send_click() {
  local cfg=$1
  local app_id=$2
  local os=$3
  local device=$4
  local now=$5
  local impression_id=$6
  local placement_id=$7
  local adapter_id=$8
  local request_id=$9
  local session_id=${10}
  local user_id=${11}
  local events="[{\"event_id\":\"$(uuid)\",\"timestamp\":\"$now\",\"impression_id\":\"$impression_id\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"ios_tv_test\",\"click_url\":\"https://example.com/click?iid=$impression_id\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"session_id\":\"$session_id\",\"user_id\":\"$user_id\",\"request_id\":\"$request_id\",\"time_to_click_ms\":$((RANDOM % 4000 + 120)),\"is_verified\":true,\"is_test_mode\":false}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/clicks" \
    --config "$cfg" \
    -d "{\"events\": $events}"
}

send_imp() {
  local cfg=$1
  local app_id=$2
  local os=$3
  local device=$4
  local fmt=$5
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local session_id=$(uuid)
  local request_id=$(uuid)
  local placement_id=$(uuid)
  local adapter_id=$(uuid)
  local user_id=$(uuid)
  local impression_id=$(uuid)
  local os_ver="16.0"
  [[ "$os" == "tvos" ]] && os_ver="17.0"
  local events="[{\"event_id\":\"$impression_id\",\"timestamp\":\"$now\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"ios_tv_test\",\"ad_unit_id\":\"${os}_${fmt}_unit\",\"ad_format\":\"$fmt\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"os_version\":\"$os_ver\",\"app_version\":\"1.0.0\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$session_id\",\"user_id\":\"$user_id\",\"request_id\":\"$request_id\",\"bid_price_usd\":$(revenue),\"ecpm_usd\":$(revenue),\"latency_ms\":$(latency),\"is_test_mode\":false}]"
  local code
  code=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/impressions" \
    --config "$cfg" \
    -d "{\"events\": $events}")

  # Occasionally emit a click for the same impression.
  if [[ "$code" == "202" ]] && [[ $((RANDOM % 6)) -eq 0 ]]; then
    : "$(send_click "$cfg" "$app_id" "$os" "$device" "$now" "$impression_id" "$placement_id" "$adapter_id" "$request_id" "$session_id" "$user_id")"
  fi

  echo "$code"
}

send_rev() {
  local cfg=$1
  local app_id=$2
  local os=$3
  local device=$4
  local fmt=$5
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local placement_id=$(uuid)
  local adapter_id=$(uuid)
  local impression_id=$(uuid)
  local rev=$(revenue)
  local events="[{\"event_id\":\"$(uuid)\",\"timestamp\":\"$now\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"ios_tv_test\",\"impression_id\":\"$impression_id\",\"revenue_type\":\"impression\",\"revenue_usd\":$rev,\"revenue_currency\":\"USD\",\"revenue_original\":$rev,\"exchange_rate\":1.0,\"ecpm_usd\":$(revenue),\"country_code\":\"US\",\"ad_format\":\"$fmt\",\"os\":\"$os\",\"is_test_mode\":false,\"reconciliation_status\":\"pending\"}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/revenue" \
    --config "$cfg" \
    -d "{\"events\": $events}"
}

SLEEP=$((60 / (EVENTS_PER_MINUTE>0?EVENTS_PER_MINUTE:1) ))
[[ $SLEEP -lt 1 ]] && SLEEP=1
log "iOS/tvOS API harness started (ios=$APP_ID_IOS, ctv=$APP_ID_CTV)"
START=$(date +%s)
ITER=0
while true; do
  ITER=$((ITER+1))
  code1=$(send_imp "$CURL_CFG_IOS" "$APP_ID_IOS" "ios" "phone" "interstitial")
  code2=$(send_imp "$CURL_CFG_CTV" "$APP_ID_CTV" "tvos" "tv" "video")
  if [[ $((ITER % 3)) -eq 0 ]]; then
    code3=$(send_rev "$CURL_CFG_IOS" "$APP_ID_IOS" "ios" "phone" "interstitial")
    code4=$(send_rev "$CURL_CFG_CTV" "$APP_ID_CTV" "tvos" "tv" "video")
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
