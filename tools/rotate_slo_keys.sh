#!/usr/bin/env bash
# Rotate SDK keys for apps in /opt/adproject/.env.slo and verify ingestion
set -euo pipefail

ENV_FILE=/opt/adproject/.env.slo
if [[ ! -f "$ENV_FILE" ]]; then echo "missing $ENV_FILE" >&2; exit 2; fi

set -a; source "$ENV_FILE"; set +a

need() { local k="$1"; if [[ -z "${!k:-}" ]]; then echo "missing $k in .env.slo" >&2; exit 2; fi; }
need API_URL
need TOKEN
need APP_ID_ANDROID
need APP_ID_IOS
need APP_ID_WEB

# Python helper to update an env var in the file
update_env() {
  local name="$1"
  local value="$2"
  python3 - "$ENV_FILE" "$name" "$value" <<'PYEND'
import sys
from pathlib import Path
env_file, name, value = sys.argv[1], sys.argv[2], sys.argv[3]
## Allow empty values so we can disable invalid apps (e.g. APP_ID_ANDROID_2=)

p = Path(env_file)
lines = p.read_text(encoding="utf-8", errors="replace").splitlines(True)
out = []
found = False
for line in lines:
    stripped = line.lstrip()
    if stripped.startswith(name + "=") or stripped.startswith("export " + name + "="):
        out.append(f"{name}={value}\n")
        found = True
    else:
        out.append(line)
if not found:
    if out and not out[-1].endswith("\n"):
        out[-1] = out[-1] + "\n"
    out.append(f"{name}={value}\n")
p.write_text("".join(out), encoding="utf-8")
PYEND
}

# Python helper to parse a JSON field from a file
parse_field() {
  local file="$1"
  local field="$2"
  python3 - "$file" "$field" <<'PYEND'
import sys, json
path = sys.argv[1]
field = sys.argv[2]
with open(path, "r", encoding="utf-8", errors="replace") as f:
    obj = json.load(f)
d = obj.get("data") or {}
print(d.get(field, "") or "")
PYEND
}

create_app_key() {
  local app_id="$1"
  local var_name="$2"
  local app_var_name="${3:-}"
  local tmp
  tmp="$(mktemp)"

  # Response includes secret; never print it.
  if ! curl -fsSL -X POST "$API_URL/api/v1/apps/$app_id/keys" \
      -H "Authorization: Bearer $TOKEN" \
      -H "Content-Type: application/json" \
      --data '{"live":true,"environment":"production"}' \
      -o "$tmp"; then
    # If the app no longer exists for this publisher (404), disable it so the generator won't emit mixed-app batches.
    code="$(curl -sS -o /dev/null -w "%{http_code}" -X POST "$API_URL/api/v1/apps/$app_id/keys" \
      -H "Authorization: Bearer $TOKEN" \
      -H "Content-Type: application/json" \
      --data '{"live":true,"environment":"production"}' || true)"
    rm -f "$tmp"
    if [[ "$code" == "404" && -n "$app_var_name" ]]; then
      echo "warn: app $app_id not found for token (HTTP 404); disabling $app_var_name and $var_name"
      update_env "$app_var_name" ""
      update_env "$var_name" ""
      return 0
    fi
    echo "failed to create key for $var_name (HTTP $code)" >&2
    exit 3
  fi

  local secret prefix last4
  secret="$(parse_field "$tmp" secret)"
  prefix="$(parse_field "$tmp" prefix)"
  last4="$(parse_field "$tmp" last4)"
  rm -f "$tmp"

  if [[ -z "$secret" ]]; then
    echo "failed to parse secret for $var_name" >&2
    exit 3
  fi

  update_env "$var_name" "$secret"

  # Back-compat: keep SDK_KEY as a default key for older tooling.
  if [[ "$var_name" == "SDK_KEY_ANDROID" ]]; then
    update_env "SDK_KEY" "$secret"
  fi

  echo "ok: updated $var_name for app $app_id ($prefix…$last4)"
}

echo "Creating fresh SDK keys..."
create_app_key "$APP_ID_ANDROID" "SDK_KEY_ANDROID"
create_app_key "$APP_ID_IOS" "SDK_KEY_IOS"
create_app_key "$APP_ID_WEB" "SDK_KEY_WEB"

# Optional secondary apps: rotate keys if they exist; otherwise disable them to prevent mixed-app batches.
if [[ -n "${APP_ID_ANDROID_2:-}" ]]; then create_app_key "$APP_ID_ANDROID_2" "SDK_KEY_ANDROID_2" "APP_ID_ANDROID_2"; fi
if [[ -n "${APP_ID_IOS_2:-}" ]]; then create_app_key "$APP_ID_IOS_2" "SDK_KEY_IOS_2" "APP_ID_IOS_2"; fi
if [[ -n "${APP_ID_UNITY:-}" ]]; then create_app_key "$APP_ID_UNITY" "SDK_KEY_UNITY" "APP_ID_UNITY"; fi
if [[ -n "${APP_ID_CTV:-}" ]]; then create_app_key "$APP_ID_CTV" "SDK_KEY_CTV" "APP_ID_CTV"; fi

echo ""
echo "Restarting slo-continuous.service..."
systemctl restart slo-continuous.service
sleep 2
systemctl is-active slo-continuous.service

echo ""
echo "Running one synthetic load cycle..."
# Run a short, bounded cycle. The underlying script defaults to 60 minutes.
timeout 75s /opt/adproject/scripts/ops/vra_synthetic_load.sh 1 20 >/tmp/synthetic_once.out 2>&1 || true

echo ""
echo "Ingestion status codes:"
grep -E "HTTP [0-9]{3}" /tmp/synthetic_once.out | tail -n 40 || echo "(no HTTP lines found)"

echo ""
echo "Sanity: generator batches should be single-app per SDK key"
set -a; source "$ENV_FILE"; set +a
tmp_payload="$(mktemp)"
python3 /opt/adproject/tools/slo_event_generator.py --count 25 > "$tmp_payload"
python3 - "$tmp_payload" <<'PYEND'
import json, sys
data=json.load(open(sys.argv[1]))

def suf(k: str) -> str:
  return '...' + k[-6:] if k else '(none)'

bad = 0
for kind in ['impressions','clicks','revenue']:
  for b in data.get(kind, []):
    k=b.get('sdk_key','')
    ev=b.get('events',[])
    apps=sorted({e.get('app_id') for e in ev if isinstance(e,dict)})
    if len(apps) > 1:
      bad += 1
      print(f"[bad] {kind} {suf(k)} apps={apps}")
if bad == 0:
  print('[ok] each SDK key batch contains a single app_id')
PYEND
rm -f "$tmp_payload"
