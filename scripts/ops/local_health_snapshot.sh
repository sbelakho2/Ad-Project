#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

# Local prod-like health snapshot helper (pre‑DO)
#
# Creates a dated evidence folder under docs/Internal/Deployment and captures:
#  - Nginx health (curl -i http://localhost/health)
#  - docker compose ps for prod stack (optional soft-fail when Docker unavailable)
#  - Optional: Redis verify via backend container (enabled by REDIS_VERIFY=1)
#
# Usage:
#   REDIS_PASSWORD=local-strong-password \
#   REDIS_VERIFY=1 \
#   bash scripts/ops/local_health_snapshot.sh
#
# Env toggles:
#   ALLOW_SUDO=1       → attempt sudo when Docker requires it
#   ALLOW_NO_DOCKER=1  → do not fail if Docker is unavailable; record a note and continue

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/infrastructure/docker-compose.prod.yml"
EVID_DIR="$ROOT_DIR/docs/Internal/Deployment/local-validation-$(date +%F)"
HEALTH_PORT="${NGINX_PORT:-8080}"
HEALTH_URL="http://localhost:${HEALTH_PORT}/health"

# Ensure we can talk to the Docker daemon. If not, optionally fallback to sudo when ALLOW_SUDO=1
can_talk_to_docker() {
  docker info >/dev/null 2>&1
}

run_compose() {
  if can_talk_to_docker; then
    docker compose -f "$COMPOSE_FILE" "$@"
  elif [[ "${ALLOW_SUDO:-}" == "1" ]]; then
    echo "[WARN] Docker not accessible. Retrying with sudo (-E)." | tee -a "$EVID_DIR/summary.txt"
    sudo -E docker compose -f "$COMPOSE_FILE" "$@"
  elif [[ "${ALLOW_NO_DOCKER:-}" == "1" ]]; then
    echo "[WARN] Docker not accessible and ALLOW_NO_DOCKER=1; skipping 'docker compose $*' and continuing." | tee -a "$EVID_DIR/summary.txt"
    return 0
  else
    echo "[ERROR] Cannot access Docker daemon (permission denied)." | tee -a "$EVID_DIR/summary.txt"
    echo "        Fix by adding your user to the 'docker' group and re-login, or rerun with ALLOW_SUDO=1." | tee -a "$EVID_DIR/summary.txt"
    echo "        Example: ALLOW_SUDO=1 REDIS_VERIFY=1 bash scripts/ops/local_health_snapshot.sh" | tee -a "$EVID_DIR/summary.txt"
    echo "        Or allow soft-fail: ALLOW_NO_DOCKER=1 bash scripts/ops/local_health_snapshot.sh" | tee -a "$EVID_DIR/summary.txt"
    exit 1
  fi
}

mkdir -p "$EVID_DIR"

echo "[INFO] Using compose file: $COMPOSE_FILE"
echo "[INFO] Evidence dir: $EVID_DIR"

echo "[STEP] Compose ps (prod-like)" | tee -a "$EVID_DIR/summary.txt"
run_compose ps | tee -a "$EVID_DIR/summary.txt" || true
echo >> "$EVID_DIR/summary.txt"

echo "[STEP] Nginx health (${HEALTH_URL})" | tee -a "$EVID_DIR/summary.txt"
if curl -fsSI "$HEALTH_URL" >/dev/null; then
  curl -i "$HEALTH_URL" | tee -a "$EVID_DIR/summary.txt"
else
  echo "Health endpoint not reachable; ensure stack is up on port ${HEALTH_PORT}." | tee -a "$EVID_DIR/summary.txt"
fi
echo >> "$EVID_DIR/summary.txt"

if [[ "${REDIS_VERIFY:-}" == "1" ]]; then
  echo "[STEP] Redis verification via backend container" | tee "$EVID_DIR/verify-redis.txt"
  echo "[INFO] Running: node dist/scripts/verifyRedis.js (inside backend container)" | tee -a "$EVID_DIR/verify-redis.txt"
  if can_talk_to_docker; then
    docker compose -f "$COMPOSE_FILE" exec backend sh -lc \
      'node dist/scripts/verifyRedis.js' | tee -a "$EVID_DIR/verify-redis.txt" || true
  elif [[ "${ALLOW_SUDO:-}" == "1" ]]; then
    sudo -E docker compose -f "$COMPOSE_FILE" exec backend sh -lc \
      'node dist/scripts/verifyRedis.js' | tee -a "$EVID_DIR/verify-redis.txt" || true
  elif [[ "${ALLOW_NO_DOCKER:-}" == "1" ]]; then
    echo "[WARN] Skipping Redis verification because Docker is unavailable (ALLOW_NO_DOCKER=1)." | tee -a "$EVID_DIR/verify-redis.txt"
  else
    echo "[ERROR] Cannot access Docker to execute Redis verify. Re-run with ALLOW_SUDO=1 or fix Docker group, or allow soft-fail with ALLOW_NO_DOCKER=1." | tee -a "$EVID_DIR/verify-redis.txt"
  fi
  echo "[NOTE] External nmap check should be executed from another machine: nmap <host-ip> -p 6379 (expect closed/filtered)" \
    >> "$EVID_DIR/verify-redis.txt"
fi

echo "[DONE] Snapshot collected under $EVID_DIR" 1>&2
