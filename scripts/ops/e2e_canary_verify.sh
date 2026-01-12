#!/usr/bin/env bash
# Close-to-production canary: prove the full pipeline works end-to-end.
#
# 1) Ingest a small batch via public SDK ingestion routes (X-SDK-Key)
# 2) Verify the data becomes visible via authenticated analytics read APIs
#    (Authorization: Bearer TOKEN)
#
# This is designed to run safely on the Hetzner test server.
#
# Maintainer notes:
# - Read endpoints are Redis-cached; this script adds a cache-busting query param on readback
#   so canary success/failure does not depend on cache invalidation correctness.
# - “Today” is computed in UTC (00:00Z → 00:00Z). Around midnight UTC, baseline changes can
#   be unrelated to the canary.
# - Analytics reads may use a replica DB when REPLICA_DATABASE_URL is configured; replication
#   lag can delay visibility even if ingestion returned 202.

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
API_KEY=${API_KEY:-}
PUBLISHER_ID=${PUBLISHER_ID:-}
SDK_KEY=${SDK_KEY:-}

# Optional per-app SDK keys (preferred)
SDK_KEY_ANDROID=${SDK_KEY_ANDROID:-}
SDK_KEY_ANDROID_2=${SDK_KEY_ANDROID_2:-}
SDK_KEY_IOS=${SDK_KEY_IOS:-}
SDK_KEY_IOS_2=${SDK_KEY_IOS_2:-}
SDK_KEY_WEB=${SDK_KEY_WEB:-}
SDK_KEY_UNITY=${SDK_KEY_UNITY:-}
SDK_KEY_CTV=${SDK_KEY_CTV:-}

COUNT=${COUNT:-25}
POLL_SECONDS=${POLL_SECONDS:-90}
POLL_INTERVAL_SECONDS=${POLL_INTERVAL_SECONDS:-5}

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }

urlencode() {
  python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))' "$1"
}

require_env() {
  local name=$1 value=${2:-}
  if [[ -z "$value" ]]; then
    log "[err] Missing env: $name" >&2
    exit 2
  fi
}

require_any_sdk_key() {
  if [[ -z "$SDK_KEY" && -z "$SDK_KEY_ANDROID" && -z "$SDK_KEY_ANDROID_2" && -z "$SDK_KEY_IOS" && -z "$SDK_KEY_IOS_2" && -z "$SDK_KEY_WEB" && -z "$SDK_KEY_UNITY" && -z "$SDK_KEY_CTV" ]]; then
    log "[err] Missing SDK key(s): set SDK_KEY or per-app SDK_KEY_*" >&2
    exit 2
  fi
}

if [[ -z "$TOKEN" && -z "$API_KEY" ]]; then
  log "[err] Missing auth for analytics readback: set TOKEN (JWT) or API_KEY (sk_*)" >&2
  exit 2
fi
require_env PUBLISHER_ID "$PUBLISHER_ID"
require_any_sdk_key

if ! command -v python3 >/dev/null 2>&1; then
  log "[err] python3 is required" >&2
  exit 2
fi

if ! command -v curl >/dev/null 2>&1; then
  log "[err] curl is required" >&2
  exit 2
fi

TMP_DIR=$(mktemp -d)
cleanup() { rm -rf "$TMP_DIR" 2>/dev/null || true; }
trap cleanup EXIT

TODAY=$(python3 - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime('%Y-%m-%d'))
PY
)

START_ISO=$(python3 - <<'PY'
from datetime import datetime, timezone
now = datetime.now(timezone.utc)
start = datetime(now.year, now.month, now.day, tzinfo=timezone.utc)
print(start.isoformat().replace('+00:00','Z'))
PY
)

END_ISO=$(python3 - <<'PY'
from datetime import datetime, timezone, timedelta
now = datetime.now(timezone.utc)
start = datetime(now.year, now.month, now.day, tzinfo=timezone.utc)
end = start + timedelta(days=1)
print(end.isoformat().replace('+00:00','Z'))
PY
)

log "[canary] API_URL=$API_URL"
log "[canary] window: $TODAY ($START_ISO -> $END_ISO)"

get_today_impressions() {
  local url resp body code cache_bust
  cache_bust=$(python3 -c 'import time; print(time.time_ns())')
  url="$API_URL/api/v1/analytics/impressions?groupBy=date&startDate=$(urlencode "$START_ISO")&endDate=$(urlencode "$END_ISO")&_cb=$(urlencode "$cache_bust")"

  if [[ -n "$API_KEY" ]]; then
    resp=$(curl -sS -w "\n%{http_code}" "$url" -H "X-Api-Key: $API_KEY")
  else
    resp=$(curl -sS -w "\n%{http_code}" "$url" -H "Authorization: Bearer $TOKEN")
  fi

  code=$(echo "$resp" | tail -n1)
  body=$(echo "$resp" | sed '$d')

  if [[ "$code" != "200" ]]; then
    log "[err] analytics readback failed (HTTP $code)" >&2
    if [[ "$code" == "401" || "$code" == "403" ]]; then
      log "[hint] Update /opt/adproject/.env.slo with a valid TOKEN (publisher JWT) or set API_KEY=sk_* for API key auth" >&2
    fi
    echo "$body" | head -n 20 >&2
    return 1
  fi

  if [[ -z "$body" ]]; then
    log "[err] analytics readback returned empty body (HTTP $code)" >&2
    return 1
  fi

  IMPR_BODY="$body" python3 - "$TODAY" <<'PY'
import json
import os
import sys

today = sys.argv[1]
raw = os.environ.get("IMPR_BODY", "")

try:
    obj = json.loads(raw)
except Exception:
    print(0)
    raise

rows = None
if isinstance(obj, dict):
    rows = obj.get("data")
    # Some endpoints wrap: { data: { data: [...] } }
    if isinstance(rows, dict) and "data" in rows:
        rows = rows.get("data")

n = 0
if isinstance(rows, list):
    for r in rows:
        if isinstance(r, dict) and r.get("date") == today:
            n = int(r.get("impressions") or 0)
            break

print(n)
PY
}

post_kind() {
  local kind="$1" json_file="$2"
  local endpoint="$API_URL/api/v1/analytics/events/$kind"
  local tsv="$TMP_DIR/${kind}.tsv"

  python3 - "$json_file" "$kind" >"$tsv" <<'PY'
import json
import sys

path = sys.argv[1]
kind = sys.argv[2]
data = json.load(open(path, "r", encoding="utf-8"))

for b in data.get(kind, []):
    sdk_key = (b.get("sdk_key") or "").strip()
    events = b.get("events") or []
    if not sdk_key or not isinstance(events, list) or not events:
        continue
    sys.stdout.write(sdk_key)
    sys.stdout.write("\t")
    sys.stdout.write(json.dumps(events, separators=(",", ":")))
    sys.stdout.write("\n")
PY

  if [[ ! -s "$tsv" ]]; then
    log "[write] $kind: no events to send (empty)" >&2
    return 0
  fi

  while IFS=$'\t' read -r sdk_key events_json; do
    [[ -z "$sdk_key" ]] && continue
    [[ -z "$events_json" ]] && continue

    local body_file resp code body
    body_file="$TMP_DIR/${kind}_body.json"
    printf '{"events":%s}' "$events_json" >"$body_file"

    resp=$(curl -sS -w "\n%{http_code}" -X POST "$endpoint" \
      -H "Content-Type: application/json" \
      -H "X-SDK-Key: $sdk_key" \
      --data-binary "@$body_file")

    code=$(echo "$resp" | tail -n1)
    body=$(echo "$resp" | sed '$d')

    if [[ "$code" != "202" ]]; then
      local sdk_key_suffix
      sdk_key_suffix=$(printf "%s" "$sdk_key" | tail -c 7)
      log "[err] ingestion failed: $kind HTTP $code (sdk_key=...$sdk_key_suffix)" >&2
      echo "$body" | head -n 30 >&2
      return 1
    fi
  done <"$tsv"
}

baseline_impressions=$(get_today_impressions)
log "[baseline] impressions(today)=$baseline_impressions"

EVENTS_JSON="$TMP_DIR/events.json"
SEED="canary-$(date +%s)"

python3 "$ROOT_DIR/tools/slo_event_generator.py" --count "$COUNT" --seed "$SEED" >"$EVENTS_JSON"

if [[ ! -s "$EVENTS_JSON" ]]; then
  log "[err] event generator produced empty output" >&2
  exit 2
fi

if ! python3 -c 'import json,sys; json.load(open(sys.argv[1],"r",encoding="utf-8"))' "$EVENTS_JSON" >/dev/null 2>&1; then
  log "[err] event generator produced invalid JSON" >&2
  head -c 400 "$EVENTS_JSON" >&2 || true
  echo >&2
  exit 2
fi

log "[write] posting impressions/clicks/revenue (seed=$SEED count=$COUNT)"
post_kind impressions "$EVENTS_JSON"
post_kind clicks "$EVENTS_JSON"
post_kind revenue "$EVENTS_JSON"

log "[read] polling analytics/impressions until it reflects new data (timeout=${POLL_SECONDS}s)"

deadline=$(( $(date +%s) + POLL_SECONDS ))
last_impressions="$baseline_impressions"

while [[ $(date +%s) -lt $deadline ]]; do
  cur=$(get_today_impressions)
  last_impressions="$cur"

  if [[ "$cur" -gt "$baseline_impressions" ]]; then
    log "[ok] impressions(today) increased: $baseline_impressions -> $cur"
    exit 0
  fi

  log "[wait] impressions(today) still $cur (baseline=$baseline_impressions)"
  sleep "$POLL_INTERVAL_SECONDS"
done

log "[fail] did not observe impressions increase within ${POLL_SECONDS}s (baseline=$baseline_impressions last=$last_impressions)" >&2
exit 1
