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
SDK_KEY_WEB=${SDK_KEY_WEB:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
APP_ID_WEB=${APP_ID_WEB:-}

DURATION_MINUTES=${1:-0}   # 0 = run indefinitely
EVENTS_PER_MINUTE=${2:-12}

require_env() {
  local name=$1 value=${2:-}
  if [[ -z "$value" ]]; then
    echo "[err] Missing env: $name" >&2
    exit 2
  fi
}
SDK_KEY_EFFECTIVE=${SDK_KEY_WEB:-$SDK_KEY}
require_env SDK_KEY "$SDK_KEY_EFFECTIVE"
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_env APP_ID_WEB "$APP_ID_WEB"

# Avoid leaking secrets via process args (ps/systemctl). Use a curl config file
# for headers instead of passing -H "X-SDK-Key: ..." on the command line.
CURL_CFG_FILE="$(mktemp)"
chmod 600 "$CURL_CFG_FILE" || true
cat >"$CURL_CFG_FILE" <<EOF
header = "Content-Type: application/json"
header = "X-SDK-Key: $SDK_KEY_EFFECTIVE"
EOF
cleanup() { rm -f "$CURL_CFG_FILE" 2>/dev/null || true; }
trap cleanup EXIT

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
random_latency() { echo "$((RANDOM % 200 + 50))"; }

send_web_click() {
  local now="$1"
  local impression_id="$2"
  local placement_id="$3"
  local adapter_id="$4"
  local request_id="$5"
  local session_id="$6"
  local user_id="$7"
  local events="[{\"event_id\":\"$(random_uuid)\",\"timestamp\":\"$now\",\"impression_id\":\"$impression_id\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$APP_ID_WEB\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"web_test\",\"click_url\":\"https://example.com/click?iid=$impression_id\",\"country_code\":\"US\",\"device_type\":\"desktop\",\"os\":\"web\",\"session_id\":\"$session_id\",\"user_id\":\"$user_id\",\"request_id\":\"$request_id\",\"time_to_click_ms\":$((RANDOM % 3000 + 120)),\"is_verified\":true,\"is_test_mode\":false}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/clicks" \
    --config "$CURL_CFG_FILE" \
    -d "{\"events\": $events}"
}

send_web_impressions() {
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local session_id=$(random_uuid)
  local request_id=$(random_uuid)
  local placement_id=$(random_uuid)
  local adapter_id=$(random_uuid)
  local user_id=$(random_uuid)
  local impression_id=$(random_uuid)
  local ad_unit_id="web_banner_unit_$(($RANDOM % 5 + 1))"
  local events="[{\"event_id\":\"$impression_id\",\"timestamp\":\"$now\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$APP_ID_WEB\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"web_test\",\"ad_unit_id\":\"$ad_unit_id\",\"ad_format\":\"banner\",\"country_code\":\"US\",\"device_type\":\"desktop\",\"os\":\"web\",\"os_version\":\"Chrome 120\",\"app_version\":\"1.0.0\",\"sdk_version\":\"1.0.0\",\"session_id\":\"$session_id\",\"user_id\":\"$user_id\",\"request_id\":\"$request_id\",\"bid_price_usd\":$(random_revenue),\"ecpm_usd\":$(random_revenue),\"latency_ms\":$(random_latency),\"is_test_mode\":false}]"
  local code
  code=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/impressions" \
    --config "$CURL_CFG_FILE" \
    -d "{\"events\": $events}")

  # Occasionally emit a click for the same impression.
  if [[ "$code" == "202" ]] && [[ $((RANDOM % 5)) -eq 0 ]]; then
    : "$(send_web_click "$now" "$impression_id" "$placement_id" "$adapter_id" "$request_id" "$session_id" "$user_id")"
  fi

  echo "$code"
}

send_web_revenue() {
  local now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local placement_id=$(random_uuid)
  local adapter_id=$(random_uuid)
  local impression_id=$(random_uuid)
  local rev=$(random_revenue)
  local events="[{\"event_id\":\"$(random_uuid)\",\"timestamp\":\"$now\",\"publisher_id\":\"$PUBLISHER_ID\",\"app_id\":\"$APP_ID_WEB\",\"placement_id\":\"$placement_id\",\"adapter_id\":\"$adapter_id\",\"adapter_name\":\"web_test\",\"impression_id\":\"$impression_id\",\"revenue_type\":\"impression\",\"revenue_usd\":$rev,\"revenue_currency\":\"USD\",\"revenue_original\":$rev,\"exchange_rate\":1.0,\"ecpm_usd\":$(random_revenue),\"country_code\":\"US\",\"ad_format\":\"banner\",\"os\":\"web\",\"is_test_mode\":false,\"reconciliation_status\":\"pending\"}]"
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API_URL/api/v1/analytics/events/revenue" \
    --config "$CURL_CFG_FILE" \
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
