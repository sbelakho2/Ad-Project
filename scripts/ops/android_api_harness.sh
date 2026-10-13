#!/usr/bin/env bash
# Android & Android TV API Harness (Test Server Only)
# Generates real analytics events tied to the Android and CTV (Android TV) app IDs.
#
# Reads /opt/adproject/.env.slo if present for:
#   API_URL, TOKEN, SDK_KEY, PUBLISHER_ID, APP_ID_ANDROID, APP_ID_CTV
#
# Usage:
#   ./scripts/ops/android_api_harness.sh [duration_minutes] [events_per_minute]

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

echo "[$(date '+%Y-%m-%d %H:%M:%S')] Initializing Android Harness..."

if [[ -f "/opt/adproject/.env.slo" ]]; then set -a; source "/opt/adproject/.env.slo"; set +a; fi
if [[ -f ".env.slo" ]]; then set -a; source ".env.slo"; set +a; fi

API_URL=${API_URL:-https://api.apexmediation.ee}
TOKEN=${TOKEN:-}
SDK_KEY=${SDK_KEY:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
APP_ID_ANDROID=${APP_ID_ANDROID:-}
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
require_env SDK_KEY "$SDK_KEY"
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_env APP_ID_ANDROID "$APP_ID_ANDROID"
require_env APP_ID_CTV "$APP_ID_CTV"

# Avoid leaking secrets via process args (ps/systemctl). Use a curl config file
# for headers instead of passing -H "Authorization: Bearer ..." on the command line.
CURL_CFG_FILE="$(mktemp)"
chmod 600 "$CURL_CFG_FILE" || true
cat >"$CURL_CFG_FILE" <<EOF
header = "Content-Type: application/json"
header = "X-SDK-Key: $SDK_KEY"
EOF
if [[ -n "$TOKEN" ]]; then
  echo "header = \"Authorization: Bearer $TOKEN\"" >>"$CURL_CFG_FILE"
fi
cleanup() { rm -f "$CURL_CFG_FILE" 2>/dev/null || true; }
trap cleanup EXIT

EVID_DIR="evidence-$(date +%F)/android"
LOG="$EVID_DIR/android_api_harness.log"
mkdir -p "$EVID_DIR"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

log "Starting Android Harness with PID $$"
log "Config: API=$API_URL EPM=$EVENTS_PER_MINUTE"

uuid() {
  printf '%04x%04x-%04x-%04x-%04x-%04x%04x%04x\n' \
    $RANDOM $RANDOM $RANDOM \
    $((RANDOM & 0x0fff | 0x4000)) \
    $((RANDOM & 0x3fff | 0x8000)) \
    $RANDOM $RANDOM $RANDOM
}

revenue() { echo "0.0$(printf '%02d' $((RANDOM % 80 + 20)))"; }

send_click() {
  local app_id=$1 os=$2 device=$3
  local now=$4
  local impression_id=$5
  local placement_id=$6
  local adapter_id=$7
  local request_id=$8
  local session_id=$9
  local user_id=${10}
  local events="[{\"event_id\":\"$(uuid)\",\"timestamp\":\"$now\",\"impression_id\":\"$impression_id\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"android_test\",\"click_url\":\"https://example.com/click?iid=$impression_id\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"session_id\":\"$session_id\",\"user_id\":\"$user_id\",\"request_id\":\"$request_id\",\"time_to_click_ms\":$((RANDOM % 4000 + 120)),\"is_verified\":true,\"is_test_mode\":false}]"
  curl -s -w "%{http_code}" -o /dev/null \
    -X POST "$API_URL/api/v1/analytics/events/clicks" \
    --config "$CURL_CFG_FILE" \
    -d "{\"events\": $events}"
}

send_imp() {
  local app_id=$1 os=$2 device=$3 fmt=$4
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local session_id=$(uuid)
  local request_id=$(uuid)
  local placement_id=$(uuid)
  local adapter_id=$(uuid)
  local user_id=$(uuid)
  local impression_id=$(uuid)
  local events="[{\"event_id\":\"$impression_id\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"adapter_name\":\"android_test\",\"ad_format\":\"$fmt\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$session_id\",\"request_id\":\"$request_id\",\"ecpm_usd\": $(revenue),\"timestamp\":\"$now\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"ad_unit_id\":\"$(uuid)\",\"os_version\":\"11.0\",\"app_version\":\"1.0.0\",\"user_id\":\"$user_id\",\"bid_price_usd\":0.05,\"latency_ms\":150,\"is_test_mode\":false}]"
  local code
  code=$(curl -s -w "%{http_code}" -o /dev/null \
    -X POST "$API_URL/api/v1/analytics/events/impressions" \
    --config "$CURL_CFG_FILE" \
    -d "{\"events\": $events}")

  # Occasionally emit a click for the same impression.
  if [[ "$code" == "202" ]] && [[ $((RANDOM % 6)) -eq 0 ]]; then
    : "$(send_click "$app_id" "$os" "$device" "$now" "$impression_id" "$placement_id" "$adapter_id" "$request_id" "$session_id" "$user_id")"
  fi

  echo "$code"
}

send_rev() {
  local app_id=$1 os=$2 device=$3 fmt=$4
  local rev=$(revenue)
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local events="[{\"event_id\":\"$(uuid)\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$app_id\",\"adapter_name\":\"android_test\",\"revenue_type\":\"impression\",\"revenue_usd\": $rev,\"ad_format\":\"$fmt\",\"country_code\":\"US\",\"device_type\":\"$device\",\"os\":\"$os\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$(uuid)\",\"request_id\":\"$(uuid)\",\"timestamp\":\"$now\",\"placement_id\":\"$(uuid)\",\"adapter_id\":\"$(uuid)\",\"ad_unit_id\":\"$(uuid)\",\"os_version\":\"11.0\",\"app_version\":\"1.0.0\",\"user_id\":\"$(uuid)\",\"bid_price_usd\":0.05,\"latency_ms\":150,\"is_test_mode\":false,\"impression_id\":\"$(uuid)\",\"revenue_currency\":\"USD\",\"revenue_original\":$rev,\"exchange_rate\":1.0,\"ecpm_usd\":$rev,\"reconciliation_status\":\"pending\"}]"
  curl -s -w "%{http_code}" -o /dev/null \
    -X POST "$API_URL/api/v1/analytics/events/revenue" \
    --config "$CURL_CFG_FILE" \
    -d "{\"events\": $events}"
}

SLEEP=$((60 / (EVENTS_PER_MINUTE>0?EVENTS_PER_MINUTE:1) ))
[[ $SLEEP -lt 1 ]] && SLEEP=1

START=$(date +%s)
ITER=0
while true; do
  ITER=$((ITER+1))
  code1=$(send_imp "$APP_ID_ANDROID" "android" "phone" "interstitial")
  code2=$(send_imp "$APP_ID_CTV" "android" "tv" "video")

  if [[ $((ITER % 3)) -eq 0 ]]; then
    code3=$(send_rev "$APP_ID_ANDROID" "android" "phone" "interstitial")
    code4=$(send_rev "$APP_ID_CTV" "android" "tv" "video")
    : "$code3" "$code4"
  fi

  if [[ $((ITER % 20)) -eq 0 ]] || [[ "$code1" != "200" && "$code1" != "201" && "$code1" != "202" ]]; then
    log "iter=$ITER codes: android_imp=$code1 androidtv_imp=$code2"
  fi

  if [[ $DURATION_MINUTES -gt 0 ]]; then
    ELAP=$(( ( $(date +%s) - START ) / 60 ))
    [[ $ELAP -ge $DURATION_MINUTES ]] && break
  fi
  sleep "$SLEEP"
done

log "Android API harness finished"