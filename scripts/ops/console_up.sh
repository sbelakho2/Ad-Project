#!/usr/bin/env bash
set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

# Bring up the staging Console/API stack via docker compose on the droplet.
# This is a convenience wrapper for checklist item 0.0.7 preflight.
#
# Usage:
#   ./scripts/ops/console_up.sh [--date YYYY-MM-DD] [--profile ui] [--compose <extra-compose.yml> ...]
#
# Notes:
# - Expects production env files to be present (see infrastructure/production/.env.backend.example).
# - Uses docker-compose prod file by default and any extra compose files passed in.

DATE_TAG="$(date +%F)"
EXTRA_COMPOSE=()
PROFILES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --date)
      DATE_TAG="$2"; shift 2 ;;
    --profile)
      PROFILES+=("$2"); shift 2 ;;
    --compose)
      EXTRA_COMPOSE+=("-f" "$2"); shift 2 ;;
    *)
      echo "[warn] Unknown arg: $1"; shift ;;
  esac
done

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT_DIR"

EVIDENCE_DIR="evidence-${DATE_TAG}/console"
mkdir -p "$EVIDENCE_DIR"

if ! command -v docker >/dev/null 2>&1; then
  echo "[err] docker not found in PATH" >&2
  exit 1
fi

COMPOSE_FILES=(
  -f docker-compose.yml
  -f infrastructure/docker-compose.prod.yml
)

# Append any extra compose files
COMPOSE_FILES+=("${EXTRA_COMPOSE[@]}")

PROFILE_FLAGS=()
if ((${#PROFILES[@]} > 0)); then
  PROFILE_FLAGS+=("--profile" "${PROFILES[*]}")
fi

echo "[run] docker compose ${COMPOSE_FILES[*]} up -d ${PROFILE_FLAGS[*]}"
set +e
docker compose "${COMPOSE_FILES[@]}" up -d ${PROFILE_FLAGS[*]} | tee "$EVIDENCE_DIR/compose-up.txt"
STATUS=$?
set -e
if [ $STATUS -ne 0 ]; then
  echo "[warn] docker compose up returned non-zero ($STATUS). Check $EVIDENCE_DIR/compose-up.txt"
fi

echo "[run] docker compose ps"
docker compose "${COMPOSE_FILES[@]}" ps | tee "$EVIDENCE_DIR/compose-ps.txt"

echo "[ok] Compose stack is up (see $EVIDENCE_DIR). Proceed with staging_console_capture.sh for evidence."
