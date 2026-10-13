#!/usr/bin/env bash
# Bootstrap continuous SLO generator with REAL data tied to a fresh dummy publisher/apps
#
# This script from your local machine will:
# 1) SSH to the App Server and create/ensure a dummy publisher + 5 apps + SDK keys
# 2) Obtain a JWT for a chosen dummy user email
# 3) Compose a .env.slo locally using returned IDs
# 4) Copy .env.slo to the Test Server and enable the systemd service
#
# Requirements on local machine:
# - ssh/scp available
# - access to ~/.ssh/hetzner-db-mac (or set APEX_HETZNER_SSH_KEY)
# - repo synced with /opt/adproject on the app server

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

APP_SERVER_IP=${APP_SERVER_IP:-46.62.252.96}
TEST_SERVER_IP=${TEST_SERVER_IP:-157.90.126.205}
SSH_KEY=${APEX_HETZNER_SSH_KEY:-$HOME/.ssh/hetzner-db-mac}
REMOTE_PROJECT_DIR=/opt/adproject
WORK_DIR=${WORK_DIR:-/tmp}

if [[ ! -f "$SSH_KEY" ]]; then
  echo "[err] SSH key not found at $SSH_KEY" >&2
  exit 1
fi

echo "[1/7] Ensuring jq on app server..."
ssh -i "$SSH_KEY" root@"$APP_SERVER_IP" -- "apt-get update -y && apt-get install -y jq >/dev/null 2>&1 || true"

echo "[2/7] Creating dummy publisher + apps + SDK keys on app server..."
scp -i "$SSH_KEY" backend/scripts/create_dummy_publisher.js root@"$APP_SERVER_IP":"$REMOTE_PROJECT_DIR"/backend/scripts/create_dummy_publisher.js >/dev/null
DEFAULT_DUMMY_EMAIL="dummy+$(date +%s)@example.com"
DUMMY_EMAIL=${DUMMY_EMAIL:-$DEFAULT_DUMMY_EMAIL}
# Optional; if omitted the create_dummy_publisher script will generate a random password
DUMMY_PASSWORD=${DUMMY_PASSWORD:-}

ssh -i "$SSH_KEY" root@"$APP_SERVER_IP" -- "\
  set -euo pipefail; \
  cd $REMOTE_PROJECT_DIR; \
  CID=\$(docker compose -f docker-compose.hetzner.yml ps -q backend); \
  if [[ -z \"\$CID\" ]]; then echo '[err] backend container not found' >&2; exit 2; fi; \
  # copy the script into the container path expected by the image (/app/backend/scripts)
  docker exec -i \"\$CID\" mkdir -p /app/backend/scripts; \
  docker cp backend/scripts/create_dummy_publisher.js \"\$CID\":/app/backend/scripts/create_dummy_publisher.js; \
  if [[ -n \"$DUMMY_PASSWORD\" ]]; then \
    docker exec -i \"\$CID\" node /app/backend/scripts/create_dummy_publisher.js \
        --email \"$DUMMY_EMAIL\" \
        --password \"$DUMMY_PASSWORD\" \
        --company 'Dummy Publisher'; \
  else \
    docker exec -i \"\$CID\" node /app/backend/scripts/create_dummy_publisher.js \
        --email \"$DUMMY_EMAIL\" \
        --company 'Dummy Publisher'; \
  fi" \
  | tee "$WORK_DIR/dummy.out"

echo "[3/7] Minting JWT in backend container for $DUMMY_EMAIL ..."
scp -i "$SSH_KEY" backend/scripts/mint_token.js root@"$APP_SERVER_IP":"$REMOTE_PROJECT_DIR"/backend/scripts/mint_token.js >/dev/null
ssh -i "$SSH_KEY" root@"$APP_SERVER_IP" -- "\
  set -euo pipefail; \
  cd $REMOTE_PROJECT_DIR; \
  CID=\$(docker compose -f docker-compose.hetzner.yml ps -q backend); \
  if [[ -z \"\$CID\" ]]; then echo '[err] backend container not found' >&2; exit 2; fi; \
  mkdir -p /opt/adproject/tmp; \
  docker exec -i \"\$CID\" mkdir -p /app/backend/scripts; \
  docker cp backend/scripts/mint_token.js \"\$CID\":/app/backend/scripts/mint_token.js; \
  docker exec -i \"\$CID\" env EMAIL=\"$DUMMY_EMAIL\" node /app/backend/scripts/mint_token.js > /opt/adproject/tmp/dummy.token"
scp -i "$SSH_KEY" root@"$APP_SERVER_IP":/opt/adproject/tmp/dummy.token "$WORK_DIR"/dummy.token >/dev/null

PUBLISHER_ID=$(grep '^PUBLISHER_ID=' "$WORK_DIR/dummy.out" | cut -d= -f2)
APP_ID_ANDROID=$(grep '^APP_ID_ANDROID=' "$WORK_DIR/dummy.out" | cut -d= -f2)
APP_ID_IOS=$(grep '^APP_ID_IOS=' "$WORK_DIR/dummy.out" | cut -d= -f2)
APP_ID_WEB=$(grep '^APP_ID_WEB=' "$WORK_DIR/dummy.out" | cut -d= -f2)
APP_ID_UNITY=$(grep '^APP_ID_UNITY=' "$WORK_DIR/dummy.out" | cut -d= -f2)
APP_ID_CTV=$(grep '^APP_ID_CTV=' "$WORK_DIR/dummy.out" | cut -d= -f2)
SDK_KEY_ANDROID=$(grep '^SDK_KEY_ANDROID=' "$WORK_DIR/dummy.out" | cut -d= -f2)
TOKEN=$(cat "$WORK_DIR/dummy.token")

if [[ -z "$PUBLISHER_ID" || -z "$APP_ID_ANDROID" || -z "$APP_ID_IOS" || -z "$APP_ID_WEB" || -z "$APP_ID_UNITY" || -z "$APP_ID_CTV" || -z "$SDK_KEY_ANDROID" || -z "$TOKEN" ]]; then
  echo "[err] Missing values in dummy.out or token; aborting." >&2
  exit 2
fi

ENV_FILE_LOCAL="$WORK_DIR/.env.slo"
cat > "$ENV_FILE_LOCAL" <<EOF
API_URL=https://api.apexmediation.ee
TOKEN=$TOKEN
SDK_KEY=$SDK_KEY_ANDROID
PUBLISHER_ID=$PUBLISHER_ID
APP_ID_ANDROID=$APP_ID_ANDROID
APP_ID_IOS=$APP_ID_IOS
APP_ID_WEB=$APP_ID_WEB
APP_ID_UNITY=$APP_ID_UNITY
APP_ID_CTV=$APP_ID_CTV
CYCLE_MINUTES=30
RATE_EPM=20
SLEEP_BETWEEN=60
EOF

echo "[4/7] .env.slo composed at $ENV_FILE_LOCAL"

echo "[5/7] Copying .env.slo to test server:/opt/adproject/.env.slo (via App Server hop) ..."
scp -i "$SSH_KEY" "$ENV_FILE_LOCAL" root@"$APP_SERVER_IP":/opt/adproject/.env.slo
ssh -i "$SSH_KEY" root@"$APP_SERVER_IP" -- "scp -i ~/.ssh/hetzner-db-mac /opt/adproject/.env.slo root@$TEST_SERVER_IP:/opt/adproject/.env.slo"

echo "[6/7] Installing/enabling systemd service on test server..."
ssh -i "$SSH_KEY" root@"$TEST_SERVER_IP" -- "\
  set -euo pipefail; \
  cp /opt/adproject/scripts/ops/systemd/slo-continuous.service.example /etc/systemd/system/slo-continuous.service; \
  systemctl daemon-reload; \
  systemctl enable --now slo-continuous.service; \
  systemctl status --no-pager slo-continuous.service || true"

echo "[7/7] Done. Verify endpoints and website in ~10 minutes:"
echo "  curl -I https://apexmediation.ee/status"
echo "  curl -I https://apexmediation.ee/slo"
echo "  curl -s https://api.apexmediation.ee/api/v1/status.json | jq . | head"
echo "  curl -s 'https://api.apexmediation.ee/api/v1/slo.json?days=30' | jq . | head"
