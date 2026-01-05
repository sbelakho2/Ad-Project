#!/usr/bin/env bash
# Continuous SLO Data Generator (Test Server Runner)
# Purpose: Keep the public SLO page populated by generating safe synthetic load
#          and exercising critical endpoints (status, SLO, SSAI) from a test host.
#
# Usage:
#   ./scripts/ops/slo_continuous_test.sh                # defaults
#   API_URL=https://api.apexmediation.ee \
#   TOKEN=... SDK_KEY=... ./scripts/ops/slo_continuous_test.sh
#
# Environment:
#   API_URL  - Base API URL (default: https://api.apexmediation.ee)
#   TOKEN    - Bearer token for protected endpoints (optional)
#   SDK_KEY  - Default SDK key header for analytics (optional if per-app SDK_KEY_* provided)
#   CYCLE_MINUTES - Minutes per synthetic cycle (default: 30)
#   RATE_EPM      - Target events per minute for synthetic load (default: 20)
#   SLEEP_BETWEEN - Pause between cycles in seconds (default: 60)

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)

# Load optional env file for SLO runs if present
if [[ -f "/opt/adproject/.env.slo" ]]; then
  set -a; source "/opt/adproject/.env.slo"; set +a
elif [[ -f "$ROOT_DIR/.env.slo" ]]; then
  set -a; source "$ROOT_DIR/.env.slo"; set +a
fi

API_URL=${API_URL:-https://api.apexmediation.ee}
TOKEN=${TOKEN:-}
SDK_KEY=${SDK_KEY:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
APP_ID_ANDROID=${APP_ID_ANDROID:-}
APP_ID_ANDROID_2=${APP_ID_ANDROID_2:-}
APP_ID_IOS=${APP_ID_IOS:-}
APP_ID_IOS_2=${APP_ID_IOS_2:-}
APP_ID_WEB=${APP_ID_WEB:-}
APP_ID_UNITY=${APP_ID_UNITY:-}
APP_ID_CTV=${APP_ID_CTV:-}

CYCLE_MINUTES=${CYCLE_MINUTES:-30}
RATE_EPM=${RATE_EPM:-20}
SLEEP_BETWEEN=${SLEEP_BETWEEN:-60}

EVIDENCE_DIR="${ROOT_DIR}/evidence-$(date +%F)/slo"
LOG_FILE="$EVIDENCE_DIR/slo_continuous_test.log"
mkdir -p "$EVIDENCE_DIR"

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

require_env() {
  local name="$1"; local val="$2"
  if [[ -z "${val}" ]]; then
    log "[err] Missing required env: $name"
    exit 2
  fi
}

warm_public_endpoints() {
  local ts
  ts=$(date +%s)
  log "[warm] GET /api/v1/status.json"
  curl -fsS "$API_URL/api/v1/status.json" -m 10 -o "$EVIDENCE_DIR/status_${ts}.json" || log "[warn] status.json failed"
  log "[warm] GET /api/v1/slo.json"
  curl -fsS "$API_URL/api/v1/slo.json" -m 10 -o "$EVIDENCE_DIR/slo_${ts}.json" || log "[warn] slo.json failed"
}

# Minimal SSAI smoke: create a session and request a manifest if available
ssai_smoke() {
  log "[ssai] Creating test SSAI session (HLS)"
  local body='{"tenantId":"test-tenant","assetId":"asset-demo","protocol":"hls"}'
  local resp id
  resp=$(curl -sS -X POST "$API_URL/api/v1/ssai/sessions" \
    -H 'Content-Type: application/json' \
    -d "$body" || true)
  id=$(echo "$resp" | grep -o '"id"\s*:\s*"[^"]\+"' | cut -d '"' -f4 || true)
  if [[ -n "$id" ]]; then
    log "[ssai] Session created: $id"
    # Try to fetch a manifest if such route exists in the API (non-fatal if 404)
    curl -fsS "$API_URL/api/v1/ssai/sessions/$id/manifest" -m 10 -o "$EVIDENCE_DIR/ssai_${id}.m3u8" \
      && log "[ssai] Manifest fetched" || log "[ssai] Manifest endpoint not available (ok)"
  else
    log "[ssai] Unable to parse session id from response"
  fi
}

main_cycle() {
  log "=============================================="
  log "SLO Continuous Cycle: ${CYCLE_MINUTES}m @ ~${RATE_EPM} epm"
  log "API: $API_URL"
  log "=============================================="

  # Warm public endpoints to keep caches fresh for the website SLO page
  warm_public_endpoints

  # Generate synthetic analytics activity for VRA and SLO rollups
  require_env PUBLISHER_ID "$PUBLISHER_ID"
  require_env APP_ID_ANDROID "$APP_ID_ANDROID"
  # Secondary app IDs are optional; if provided they will receive traffic
  # and make the test more life-like across multiple apps under same tenant.
  APP_ID_ANDROID_2="$APP_ID_ANDROID_2" 
  require_env APP_ID_IOS "$APP_ID_IOS"
  APP_ID_IOS_2="$APP_ID_IOS_2"
  require_env APP_ID_WEB "$APP_ID_WEB"
  # Pass env explicitly to child
  (
    export API_URL TOKEN SDK_KEY PUBLISHER_ID
    export APP_ID_ANDROID APP_ID_ANDROID_2 APP_ID_IOS APP_ID_IOS_2 APP_ID_WEB
    "$ROOT_DIR/scripts/ops/vra_synthetic_load.sh" "$CYCLE_MINUTES" "$RATE_EPM"
  ) || log "[warn] synthetic load returned non-zero"

  # SSAI smoke (non-fatal)
  ssai_smoke || true

  # Warm again after load to push new SLO aggregates into cache
  warm_public_endpoints

  log "[sleep] Sleeping ${SLEEP_BETWEEN}s before next cycle"
  sleep "$SLEEP_BETWEEN"
}

log "[start] Continuous SLO generator starting (PID $$)"
log "[params] API_URL=$API_URL CYCLE_MINUTES=$CYCLE_MINUTES RATE_EPM=$RATE_EPM SLEEP_BETWEEN=$SLEEP_BETWEEN"

while true; do
  main_cycle
done
