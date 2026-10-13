#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

ROOT_DIR="$(cd "$(dirname "$0")"/../.. && pwd)"
cd "$ROOT_DIR"

ssh_db() {
  # scripts/ops/hetzner_ssh.sh prints a '[run] ...' line to stdout; strip it for machine parsing.
  ./scripts/ops/hetzner_ssh.sh db -- "$1" | sed '/^\[run\] /d'
}

BASE_URL="${BASE_URL:-https://api.apexmediation.ee}"
EMAIL="${EMAIL:-}"
PASSWORD="${PASSWORD:-}"
PUBLISHER_ID="${PUBLISHER_ID:-}"
FROM_ISO="${FROM_ISO:-}"
TO_ISO="${TO_ISO:-}"
HOURS="${HOURS:-2}"
RECENT_RAW_DAYS="${RECENT_RAW_DAYS:-2}"
ENV_LIVE_PATH="${ENV_LIVE_PATH:-$ROOT_DIR/.env.live}"

usage() {
  cat <<'USAGE'
Verify prod consistency: analytics ingestion (stage→facts→revenue API) + VRA (DB→/recon/overview).

Usage:
  scripts/ops/verify_prod_publisher_consistency.sh [options]

Options:
  --base-url URL          Backend base URL (default https://api.apexmediation.ee)
  --email EMAIL           Login email (defaults from .env.live if present)
  --password PASS         Login password (defaults from .env.live if present)
  --publisher-id UUID     Publisher id (optional; auto-resolved by email via DB if omitted)
  --from ISO              Window start (ISO-8601, UTC recommended)
  --to ISO                Window end (ISO-8601, UTC recommended)
  --hours N               If --from/--to not provided, use last N hours (default 2)

Env vars (alternatives):
  BASE_URL, EMAIL, PASSWORD, PUBLISHER_ID, FROM_ISO, TO_ISO, HOURS

Notes:
- Requires SSH access via scripts/ops/hetzner_ssh.sh (db server) and curl locally.
- Does not print tokens/passwords.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url) BASE_URL="$2"; shift 2 ;;
    --email) EMAIL="$2"; shift 2 ;;
    --password) PASSWORD="$2"; shift 2 ;;
    --publisher-id) PUBLISHER_ID="$2"; shift 2 ;;
    --from) FROM_ISO="$2"; shift 2 ;;
    --to) TO_ISO="$2"; shift 2 ;;
    --hours) HOURS="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "[err] unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

# Best-effort: load .env.live if present
if [[ ( -z "${EMAIL}" || -z "${PASSWORD}" ) && -f "$ENV_LIVE_PATH" ]]; then
  # shellcheck disable=SC1090
  source "$ROOT_DIR/scripts/ops/source_env.sh" "$ENV_LIVE_PATH" >/dev/null
  EMAIL="${EMAIL:-${LIVE_CONSOLE_EMAIL:-${CONSOLE_ADMIN_EMAIL:-}}}"
  PASSWORD="${PASSWORD:-${LIVE_CONSOLE_PASSWORD:-${LIVE_CONSOLE_PASS:-${CONSOLE_ADMIN_PASSWORD:-}}}}"
fi

if [[ -z "$EMAIL" || -z "$PASSWORD" ]]; then
  echo "[err] Missing --email/--password (or set in .env.live via LIVE_CONSOLE_EMAIL + LIVE_CONSOLE_PASSWORD)" >&2
  exit 2
fi

if [[ -z "$FROM_ISO" || -z "$TO_ISO" ]]; then
  NOW_UTC=$(python3 - <<'PY'
import datetime
print(datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds').replace('+00:00','Z'))
PY
)
  FROM_ISO=$(python3 - <<PY
import datetime
from datetime import timezone
now=datetime.datetime.now(timezone.utc)
delta=datetime.timedelta(hours=float("$HOURS"))
print((now-delta).isoformat(timespec='seconds').replace('+00:00','Z'))
PY
)
  TO_ISO="$NOW_UTC"
fi

# Login to get bearer token (avoid printing secrets; handle non-2xx and non-JSON responses)
LOGIN_PAYLOAD=$(python3 - <<PY
import json
print(json.dumps({'email': "${EMAIL}", 'password': "${PASSWORD}"}))
PY
)

LOGIN_RAW=$(curl -sS -X POST "${BASE_URL}/api/v1/auth/login" \
  -H 'Content-Type: application/json' \
  --data "$LOGIN_PAYLOAD" \
  -w $'\n__HTTP__%{http_code}')

LOGIN_HTTP=$(echo "$LOGIN_RAW" | tail -n 1 | sed 's/^__HTTP__//')
LOGIN_BODY=$(echo "$LOGIN_RAW" | sed '$d')

if [[ "$LOGIN_HTTP" != "200" ]]; then
  echo "[err] /auth/login returned HTTP $LOGIN_HTTP" >&2
  echo "[hint] Check BASE_URL ($BASE_URL) and credentials (not printed)." >&2
  echo "[hint] Response (first 200 chars): $(echo "$LOGIN_BODY" | tr '\n' ' ' | head -c 200)" >&2
  exit 10
fi

TOKEN=$(printf %s "$LOGIN_BODY" | jq -er '
  if (.success // false) != true then "__LOGIN_FAILED__" else
    if (.data.twofaRequired // false) == true then "__2FA_REQUIRED__" else
      (.data.token // "")
    end
  end
') || TOKEN="__NON_JSON__"

if [[ "$TOKEN" == "__NON_JSON__" ]]; then
  echo "[err] /auth/login did not return JSON" >&2
  echo "[hint] Response (first 200 chars): $(echo "$LOGIN_BODY" | tr '\n' ' ' | head -c 200)" >&2
  exit 10
fi
if [[ "$TOKEN" == "__LOGIN_FAILED__" ]]; then
  echo "[err] Login failed (success=false)." >&2
  exit 10
fi

if [[ "$TOKEN" == "__2FA_REQUIRED__" ]]; then
  echo "[err] Login requires 2FA step-up; provide a CONSOLE_TOKEN workflow instead." >&2
  exit 10
fi
if [[ -z "$TOKEN" ]]; then
  echo "[err] Failed to obtain JWT (empty token)." >&2
  exit 10
fi

# Resolve publisher id if needed (via DB)
if [[ -z "$PUBLISHER_ID" ]]; then
  EMAIL_SQL=$(python3 - <<PY
import sys
print("$EMAIL".replace("'","''"))
PY
)
  PUBLISHER_ID=$(ssh_db \
    "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 -c \"SELECT publisher_id::text FROM users WHERE email = '${EMAIL_SQL}' LIMIT 1;\"" \
    | tr -d '\r\n' | grep -Eo '[0-9a-fA-F-]{36}' | head -n 1)
fi

if [[ -z "$PUBLISHER_ID" ]]; then
  echo "[err] Could not resolve publisher_id for email=$EMAIL" >&2
  exit 2
fi

# Fetch DB aggregates as JSON (via db server)
PID_SQL=$(python3 - <<PY
print("$PUBLISHER_ID".replace("'","''"))
PY
)
FROM_SQL=$(python3 - <<PY
print("$FROM_ISO".replace("'","''"))
PY
)
TO_SQL=$(python3 - <<PY
print("$TO_ISO".replace("'","''"))
PY
)

has_column() {
  local table="$1"
  local column="$2"
  ssh_db "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 -c \"SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='${table}' AND column_name='${column}' LIMIT 1;\"" \
    | tail -n 1 | tr -d '\r\n'
}

has_table() {
  local table="$1"
  ssh_db "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 -c \"SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name='${table}' LIMIT 1;\"" \
    | tail -n 1 | tr -d '\r\n'
}

# Align DB-side aggregates with the API reporting semantics (exclude test-mode data when possible).
IMP_TEST_FILTER=""
CLK_TEST_FILTER=""
REV_TEST_FILTER=""
if [[ "$(has_column analytics_impressions is_test_mode)" == "1" ]]; then
  IMP_TEST_FILTER=" AND is_test_mode = false"
fi
if [[ "$(has_column analytics_clicks is_test_mode)" == "1" ]]; then
  CLK_TEST_FILTER=" AND is_test_mode = false"
fi
if [[ "$(has_column analytics_revenue_events is_test_mode)" == "1" ]]; then
  REV_TEST_FILTER=" AND is_test_mode = false"
fi

# Detect revenue_events schema so VRA checks don't hard-fail on older DBs.
HAS_REV_TS=$(ssh_db "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 -c \"SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='revenue_events' AND column_name='timestamp' LIMIT 1;\"" | tail -n 1 | tr -d '\r\n')
HAS_REV_USD=$(ssh_db "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 -c \"SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='revenue_events' AND column_name='revenue_usd' LIMIT 1;\"" | tail -n 1 | tr -d '\r\n')

VRA_EXPECTED_MODE="event_date/revenue"
if [[ "$HAS_REV_TS" == "1" && "$HAS_REV_USD" == "1" ]]; then
  VRA_EXPECTED_MODE="timestamp/revenue_usd"
fi

EXPECTED_ROWS_SQL="(SELECT COUNT(*)::bigint FROM revenue_events WHERE publisher_id::text = '${PID_SQL}' AND event_date >= ('${FROM_SQL}')::date AND event_date < ('${TO_SQL}')::date)"
EXPECTED_USD_SQL="(SELECT COALESCE(ROUND(SUM(revenue)::numeric, 6), 0)::text FROM revenue_events WHERE publisher_id::text = '${PID_SQL}' AND event_date >= ('${FROM_SQL}')::date AND event_date < ('${TO_SQL}')::date)"
if [[ "$VRA_EXPECTED_MODE" == "timestamp/revenue_usd" ]]; then
  EXPECTED_ROWS_SQL="(SELECT COUNT(*)::bigint FROM revenue_events WHERE publisher_id::text = '${PID_SQL}' AND \"timestamp\" >= '${FROM_SQL}'::timestamptz AND \"timestamp\" < '${TO_SQL}'::timestamptz)"
  EXPECTED_USD_SQL="(SELECT COALESCE(ROUND(SUM(revenue_usd)::numeric, 6), 0)::text FROM revenue_events WHERE publisher_id::text = '${PID_SQL}' AND \"timestamp\" >= '${FROM_SQL}'::timestamptz AND \"timestamp\" < '${TO_SQL}'::timestamptz)"
fi

# Reporting semantics depend on whether rollups exist.
HAS_ROLLUPS="$(has_table analytics_metrics_rollups)"

if [[ "$HAS_ROLLUPS" == "1" ]]; then
  DB_JSON=$(ssh_db "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 <<'SQL'
WITH
  bounds AS (
    SELECT '${FROM_SQL}'::timestamptz AS from_ts, '${TO_SQL}'::timestamptz AS to_ts
  ),
  split AS (
    SELECT
      from_ts,
      to_ts,
      (date_trunc('day', (to_ts AT TIME ZONE 'UTC') - ('${RECENT_RAW_DAYS} days')::interval) AT TIME ZONE 'UTC') AS split_ts
    FROM bounds
  ),
  windows AS (
    SELECT
      from_ts,
      to_ts,
      split_ts,
      LEAST(to_ts, split_ts) AS rollup_end_excl,
      GREATEST(from_ts, split_ts) AS raw_start
    FROM split
  ),
  rollups AS (
    SELECT
      COALESCE(SUM(total_revenue), 0) AS revenue,
      COALESCE(SUM(impression_count), 0)::bigint AS impressions,
      COALESCE(SUM(click_count), 0)::bigint AS clicks,
      COALESCE(SUM(filled_count), 0)::bigint AS filled
    FROM analytics_metrics_rollups m
    JOIN windows w ON true
    WHERE m.publisher_id::text = '${PID_SQL}'
      AND m.bucket_date >= ((w.from_ts AT TIME ZONE 'UTC')::date)
      AND m.bucket_date < ((w.rollup_end_excl AT TIME ZONE 'UTC')::date)
  ),
  raw AS (
    SELECT
      (SELECT COUNT(*)::bigint FROM analytics_impressions i JOIN windows w ON true
        WHERE i.publisher_id::text = '${PID_SQL}'
          AND i.observed_at >= w.raw_start
          AND i.observed_at < w.to_ts${IMP_TEST_FILTER}
      ) AS impressions,
      (SELECT COUNT(*)::bigint FROM analytics_clicks c JOIN windows w ON true
        WHERE c.publisher_id::text = '${PID_SQL}'
          AND c.observed_at >= w.raw_start
          AND c.observed_at < w.to_ts${CLK_TEST_FILTER}
      ) AS clicks,
      (SELECT COALESCE(ROUND(SUM(r.revenue_usd)::numeric, 6), 0)::text FROM analytics_revenue_events r JOIN windows w ON true
        WHERE r.publisher_id::text = '${PID_SQL}'
          AND r.observed_at >= w.raw_start
          AND r.observed_at < w.to_ts${REV_TEST_FILTER}
      ) AS revenue_usd
  )
SELECT json_build_object(
  'publisherId', '${PID_SQL}',
  'window', json_build_object('from', '${FROM_SQL}', 'to', '${TO_SQL}'),
  'vraExpectedMode', '${VRA_EXPECTED_MODE}',
  'analytics', json_build_object(
    'stage', json_build_object(
      'impressions', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_impressions_stage WHERE publisher_id::text = '${PID_SQL}'),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_impressions_stage WHERE publisher_id::text = '${PID_SQL}'), '')
      ),
      'clicks', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_clicks_stage WHERE publisher_id::text = '${PID_SQL}'),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_clicks_stage WHERE publisher_id::text = '${PID_SQL}'), '')
      ),
      'revenue', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_revenue_events_stage WHERE publisher_id::text = '${PID_SQL}'),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_revenue_events_stage WHERE publisher_id::text = '${PID_SQL}'), '')
      )
    ),
    'factsWindow', json_build_object(
      'impressions', json_build_object(
        'rows', ((SELECT impressions FROM rollups) + (SELECT impressions FROM raw)),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_impressions WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz${IMP_TEST_FILTER}), '')
      ),
      'clicks', json_build_object(
        'rows', ((SELECT clicks FROM rollups) + (SELECT clicks FROM raw)),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_clicks WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz${CLK_TEST_FILTER}), '')
      ),
      'revenue', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_revenue_events WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}),
        'usd', ((SELECT COALESCE(ROUND((SELECT revenue FROM rollups)::numeric,6),0)::text FROM rollups)::numeric + (SELECT revenue_usd::numeric FROM raw))::text,
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_revenue_events WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}), '')
      )
    ),
    'rawFactRowsWindow', json_build_object(
      'impressions', (SELECT COUNT(*)::bigint FROM analytics_impressions WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz),
      'clicks', (SELECT COUNT(*)::bigint FROM analytics_clicks WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz),
      'revenue_events', (SELECT COUNT(*)::bigint FROM analytics_revenue_events WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz)
    )
  ),
  'vra', json_build_object(
    'expectedWindow', json_build_object(
      'rows', ${EXPECTED_ROWS_SQL},
      'usd',  ${EXPECTED_USD_SQL}
    ),
    'paidWindow', json_build_object(
      'rows', (SELECT COALESCE(SUM(impressions),0)::bigint FROM recon_statements_norm WHERE publisher_id::text = '${PID_SQL}' AND event_date >= ('${FROM_SQL}')::date AND event_date < ('${TO_SQL}')::date),
      'usd', (SELECT COALESCE(ROUND(SUM(paid)::numeric, 6), 0)::text FROM recon_statements_norm WHERE publisher_id::text = '${PID_SQL}' AND event_date >= ('${FROM_SQL}')::date AND event_date < ('${TO_SQL}')::date)
    )
  )
)::text;
SQL" | tail -n 1)
else
  DB_JSON=$(ssh_db "sudo -u postgres psql -d adproject -P pager=off -At -v ON_ERROR_STOP=1 <<'SQL'
SELECT json_build_object(
  'publisherId', '${PID_SQL}',
  'window', json_build_object('from', '${FROM_SQL}', 'to', '${TO_SQL}'),
  'vraExpectedMode', '${VRA_EXPECTED_MODE}',
  'analytics', json_build_object(
    'stage', json_build_object(
      'impressions', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_impressions_stage WHERE publisher_id::text = '${PID_SQL}'),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_impressions_stage WHERE publisher_id::text = '${PID_SQL}'), '')
      ),
      'clicks', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_clicks_stage WHERE publisher_id::text = '${PID_SQL}'),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_clicks_stage WHERE publisher_id::text = '${PID_SQL}'), '')
      ),
      'revenue', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_revenue_events_stage WHERE publisher_id::text = '${PID_SQL}'),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_revenue_events_stage WHERE publisher_id::text = '${PID_SQL}'), '')
      )
    ),
    'factsWindow', json_build_object(
      'impressions', json_build_object(
        'rows', (SELECT COALESCE(COUNT(DISTINCT r.impression_id), 0)::bigint FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}), '')
      ),
      'clicks', json_build_object(
        'rows', (SELECT COALESCE(COUNT(*) FILTER (WHERE r.revenue_type = 'click'), 0)::bigint FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}), '')
      ),
      'revenue', json_build_object(
        'rows', (SELECT COUNT(*)::bigint FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}),
        'usd', (SELECT COALESCE(ROUND(SUM(r.revenue_usd)::numeric, 6), 0)::text FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}),
        'maxObservedAt', COALESCE((SELECT MAX(observed_at)::text FROM analytics_revenue_events r WHERE r.publisher_id::text = '${PID_SQL}' AND r.observed_at >= '${FROM_SQL}'::timestamptz AND r.observed_at < '${TO_SQL}'::timestamptz${REV_TEST_FILTER}), '')
      )
    ),
    'rawFactRowsWindow', json_build_object(
      'impressions', (SELECT COUNT(*)::bigint FROM analytics_impressions WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz),
      'clicks', (SELECT COUNT(*)::bigint FROM analytics_clicks WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz),
      'revenue_events', (SELECT COUNT(*)::bigint FROM analytics_revenue_events WHERE publisher_id::text = '${PID_SQL}' AND observed_at >= '${FROM_SQL}'::timestamptz AND observed_at < '${TO_SQL}'::timestamptz)
    )
  ),
  'vra', json_build_object(
    'expectedWindow', json_build_object(
      'rows', ${EXPECTED_ROWS_SQL},
      'usd',  ${EXPECTED_USD_SQL}
    ),
    'paidWindow', json_build_object(
      'rows', (SELECT COALESCE(SUM(impressions),0)::bigint FROM recon_statements_norm WHERE publisher_id::text = '${PID_SQL}' AND event_date >= ('${FROM_SQL}')::date AND event_date < ('${TO_SQL}')::date),
      'usd', (SELECT COALESCE(ROUND(SUM(paid)::numeric, 6), 0)::text FROM recon_statements_norm WHERE publisher_id::text = '${PID_SQL}' AND event_date >= ('${FROM_SQL}')::date AND event_date < ('${TO_SQL}')::date)
    )
  )
)::text;
SQL" | tail -n 1)
fi

# Fetch API results
REV_SUMMARY_JSON=$(curl -fsSL "${BASE_URL}/api/v1/revenue/summary?startDate=$(python3 - <<PY
import urllib.parse
print(urllib.parse.quote("$FROM_ISO"))
PY
)&endDate=$(python3 - <<PY
import urllib.parse
print(urllib.parse.quote("$TO_ISO"))
PY
)" -H "Authorization: Bearer ${TOKEN}")

RECON_OVERVIEW_JSON=$(curl -fsSL "${BASE_URL}/api/v1/recon/overview?from=$(python3 - <<PY
import urllib.parse
print(urllib.parse.quote("$FROM_ISO"))
PY
)&to=$(python3 - <<PY
import urllib.parse
print(urllib.parse.quote("$TO_ISO"))
PY
)" -H "Authorization: Bearer ${TOKEN}")

# Compare + print a compact report
python3 - <<PY
import json
from decimal import Decimal

db=json.loads('''$DB_JSON''')
rev=json.loads('''$REV_SUMMARY_JSON''')
recon=json.loads('''$RECON_OVERVIEW_JSON''')

assert rev.get('success') is True, rev
assert recon.get('success') is True, recon

rev_data=rev.get('data') or {}
recon_data=recon.get('data') or {}

facts=db['analytics']['factsWindow']

exp_impr=int(facts['impressions']['rows'])
exp_clicks=int(facts['clicks']['rows'])
exp_rev=Decimal(str(facts['revenue']['usd']))

api_impr=int(rev_data.get('totalImpressions',0))
api_clicks=int(rev_data.get('totalClicks',0))
api_rev=Decimal(str(rev_data.get('totalRevenue',0)))

stage=db['analytics']['stage']
stage_rows=int(stage['impressions']['rows'])+int(stage['clicks']['rows'])+int(stage['revenue']['rows'])

ok=True

def near(a: Decimal, b: Decimal, eps: Decimal = Decimal('0.000001')):
    return abs(a-b) <= eps

if api_impr != exp_impr:
    ok=False
if api_clicks != exp_clicks:
    ok=False
if not near(api_rev, exp_rev):
    ok=False

# VRA totals comparison (API computed from same DB, but validates auth/window wiring)
vra_db_exp=Decimal(str(db['vra']['expectedWindow']['usd']))
vra_db_paid=Decimal(str(db['vra']['paidWindow']['usd']))
vra_api_tot=recon_data.get('totals') or {}
vra_api_exp=Decimal(str(vra_api_tot.get('expected',0)))
vra_api_paid=Decimal(str(vra_api_tot.get('paid',0)))

vra_ok=True
if not near(vra_api_exp, vra_db_exp):
    vra_ok=False
if not near(vra_api_paid, vra_db_paid):
    vra_ok=False

print('=== Prod publisher consistency ===')
print('publisherId:', db['publisherId'])
print('window:', db['window']['from'], '→', db['window']['to'])
print('')
print('[analytics] stage rows total:', stage_rows, '(should usually be ~0)')
print('[analytics] DB facts window: impressions=%d clicks=%d revenue_usd=%s' % (exp_impr, exp_clicks, exp_rev))
print('[analytics] API /revenue/summary: impressions=%d clicks=%d revenue_usd=%s' % (api_impr, api_clicks, api_rev))
print('analytics_match:', 'OK' if ok else 'MISMATCH')
print('')
print('[vra] DB window: expected_usd=%s paid_usd=%s' % (vra_db_exp, vra_db_paid))
print('[vra] API /recon/overview totals: expected=%s paid=%s' % (vra_api_exp, vra_api_paid))
print('vra_match:', 'OK' if vra_ok else 'MISMATCH')
print('')
if not ok:
    print('!! analytics mismatch details')
    print('  impressions: db=%d api=%d' % (exp_impr, api_impr))
    print('  clicks:      db=%d api=%d' % (exp_clicks, api_clicks))
    print('  revenue_usd: db=%s api=%s' % (exp_rev, api_rev))
    raw=db.get('analytics',{}).get('rawFactRowsWindow',{}) or {}
    if raw:
        print('  raw_fact_rows_window:', raw)

if stage_rows > 0:
    print('!! staging backlog detected (ingestion drain lag)')
    print('  stage.impressions.rows=', stage['impressions']['rows'], 'maxObservedAt=', stage['impressions']['maxObservedAt'])
    print('  stage.clicks.rows=', stage['clicks']['rows'], 'maxObservedAt=', stage['clicks']['maxObservedAt'])
    print('  stage.revenue.rows=', stage['revenue']['rows'], 'maxObservedAt=', stage['revenue']['maxObservedAt'])

if not (ok and vra_ok):
    raise SystemExit(1)
PY

echo "[ok] consistency checks passed"
