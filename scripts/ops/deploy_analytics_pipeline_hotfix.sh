#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT_DIR"

echo "== Deploy: backend analytics pipeline hotfix =="

# 1) Patch backend on app server, rebuild, restart.
# Payloads are transported as single-quoted heredocs (same pattern as hetzner_rollback.sh);
# the previous inline double-quoted form was unparseable because the payload contains
# double quotes and nested heredocs.
payload_app=$(cat <<'REMOTE'
set -euo pipefail
FILE=/opt/adproject/backend/src/services/analyticsService.ts
python3 - <<'PY'
from pathlib import Path
p=Path('/opt/adproject/backend/src/services/analyticsService.ts')
text=p.read_text(encoding='utf-8')
if 'ON CONFLICT DO NOTHING' in text:
    print('backend already patched')
else:
    needle = "    if (hasUniqueOn(['event_id'])) {\n      return 'ON CONFLICT (\\\"event_id\\\") DO NOTHING';\n    }\n    return '';\n"
    replacement = "    if (hasUniqueOn(['event_id'])) {\n      return 'ON CONFLICT (\\\"event_id\\\") DO NOTHING';\n    }\n\n    // Safety fallback: if the target has unique constraints we didn't detect (e.g. partition-local\n    // unique indexes), inserting a batch with duplicate rows can wedge the stage->fact merge.\n    // `ON CONFLICT DO NOTHING` (without a conflict target) is valid and will ignore any unique\n    // violations the insert encounters.\n    return 'ON CONFLICT DO NOTHING';\n"
    if needle not in text:
        raise SystemExit('backend patch failed: expected block not found')
    p.write_text(text.replace(needle, replacement), encoding='utf-8')
    print('backend patched')
PY

cd /opt/adproject/backend
npm run build
pm2 restart backend
pm2 status | sed -n '1,35p'
REMOTE
)
./scripts/ops/hetzner_ssh.sh app -- "$payload_app"

# 2) Patch harness scripts on test server and restart services
payload_test=$(cat <<'REMOTE'
set -euo pipefail
python3 - <<'PY'
from pathlib import Path

def patch(path: str, old: str, new: str):
    p=Path(path)
    text=p.read_text(encoding='utf-8')
    if new in text:
        print('already patched:', path)
        return
    if old not in text:
        raise SystemExit(f'patch failed (pattern not found): {path}')
    p.write_text(text.replace(old,new), encoding='utf-8')
    print('patched:', path)

# web_sdk_harness: prefer SDK_KEY_WEB
patch(
  '/opt/adproject/scripts/ops/web_sdk_harness.sh',
  'SDK_KEY=${SDK_KEY:-}\nPUBLISHER_ID=${PUBLISHER_ID:-}\nAPP_ID_WEB=${APP_ID_WEB:-}\n',
  'SDK_KEY=${SDK_KEY:-}\nSDK_KEY_WEB=${SDK_KEY_WEB:-}\nPUBLISHER_ID=${PUBLISHER_ID:-}\nAPP_ID_WEB=${APP_ID_WEB:-}\n'
)
patch(
  '/opt/adproject/scripts/ops/web_sdk_harness.sh',
  'require_env SDK_KEY "$SDK_KEY"\n',
  'SDK_KEY_EFFECTIVE=${SDK_KEY_WEB:-$SDK_KEY}\nrequire_env SDK_KEY "$SDK_KEY_EFFECTIVE"\n'
)
patch(
  '/opt/adproject/scripts/ops/web_sdk_harness.sh',
  'header = "X-SDK-Key: $SDK_KEY"\n',
  'header = "X-SDK-Key: $SDK_KEY_EFFECTIVE"\n'
)

# NOTE: ios_tv_api_harness patching is more invasive; we only apply it if it matches the expected original.
ios_path = '/opt/adproject/scripts/ops/ios_tv_api_harness.sh'
ios_text = Path(ios_path).read_text(encoding='utf-8')
if 'SDK_KEY_IOS_EFFECTIVE' in ios_text and 'SDK_KEY_CTV_EFFECTIVE' in ios_text:
    print('already patched:', ios_path)
else:
    # Bail out if file doesn't look like expected baseline.
    if 'SDK_KEY_IOS=' in ios_text or 'CURL_CFG_IOS=' in ios_text:
        raise SystemExit('ios_tv_api_harness is in an unexpected intermediate state; patch manually')

    ios_text = ios_text.replace(
        'SDK_KEY=${SDK_KEY:-}\nPUBLISHER_ID=',
        'SDK_KEY=${SDK_KEY:-}\nSDK_KEY_IOS=${SDK_KEY_IOS:-}\nSDK_KEY_CTV=${SDK_KEY_CTV:-}\nPUBLISHER_ID='
    )
    ios_text = ios_text.replace(
        'require_env SDK_KEY "$SDK_KEY"\n',
        'SDK_KEY_IOS_EFFECTIVE=${SDK_KEY_IOS:-$SDK_KEY}\nSDK_KEY_CTV_EFFECTIVE=${SDK_KEY_CTV:-$SDK_KEY}\nrequire_env SDK_KEY_IOS "$SDK_KEY_IOS_EFFECTIVE"\nrequire_env SDK_KEY_CTV "$SDK_KEY_CTV_EFFECTIVE"\n'
    )

    # Replace single curl cfg with two.
    ios_text = ios_text.replace(
        'CURL_CFG_FILE="$(mktemp)"\nchmod 600 "$CURL_CFG_FILE" || true\ncat >"$CURL_CFG_FILE" <<EOF\nheader = "Content-Type: application/json"\nheader = "X-SDK-Key: $SDK_KEY"\nEOF',
        'CURL_CFG_IOS="$(mktemp)"\nCURL_CFG_CTV="$(mktemp)"\nchmod 600 "$CURL_CFG_IOS" "$CURL_CFG_CTV" || true\ncat >"$CURL_CFG_IOS" <<EOF\nheader = "Content-Type: application/json"\nheader = "X-SDK-Key: $SDK_KEY_IOS_EFFECTIVE"\nEOF\ncat >"$CURL_CFG_CTV" <<EOF\nheader = "Content-Type: application/json"\nheader = "X-SDK-Key: $SDK_KEY_CTV_EFFECTIVE"\nEOF'
    )
    ios_text = ios_text.replace(
        'echo "header = \"Authorization: Bearer $TOKEN\"" >>"$CURL_CFG_FILE"',
        'echo "header = \"Authorization: Bearer $TOKEN\"" >>"$CURL_CFG_IOS"\necho "header = \"Authorization: Bearer $TOKEN\"" >>"$CURL_CFG_CTV"'
    )
    ios_text = ios_text.replace(
        'cleanup() { rm -f "$CURL_CFG_FILE" 2>/dev/null || true; }',
        'cleanup() { rm -f "$CURL_CFG_IOS" "$CURL_CFG_CTV" 2>/dev/null || true; }'
    )

    # Default all requests to iOS cfg; tvOS calls will be corrected by setting CURL_CFG_FILE.
    ios_text = ios_text.replace('--config "$CURL_CFG_FILE"', '--config "${CURL_CFG_FILE:-$CURL_CFG_IOS}"')

    Path(ios_path).write_text(ios_text, encoding='utf-8')
    print('patched:', ios_path)
PY

systemctl restart web-sdk-harness.service ios-tv-api-harness.service slo-continuous.service
sleep 1
systemctl --no-pager --full status web-sdk-harness.service ios-tv-api-harness.service | sed -n '1,80p'
REMOTE
)
./scripts/ops/hetzner_ssh.sh test -- "$payload_test"

echo "== Deploy complete =="
