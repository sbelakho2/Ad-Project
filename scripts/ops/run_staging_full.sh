#!/usr/bin/env bash
set -euo pipefail

# Orchestrate the full staging run (0.0.7–0.0.12) and zip evidence for 0.0.13
# Usage:
#   SMTP_PASS=re_xxx \
#   BASE_URL=https://api.apexmediation.ee \
#   ./scripts/ops/run_staging_full.sh 2025-12-08

DATE_TAG="${1:-$(date +%F)}"
ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)
EVIDENCE_DIR="${ROOT_DIR}/evidence-${DATE_TAG}"

echo "[run] Staging orchestration start for ${DATE_TAG}"
mkdir -p "${EVIDENCE_DIR}/"{console,website,billing,vra,cron,soak,sdk-logs}

# 0) Try to source production backend env for DATABASE_URL, etc., if present
ENV_PATH="${ROOT_DIR}/infrastructure/production/.env.backend"
if [ -f "$ENV_PATH" ]; then
  echo "[run] Sourcing env from $ENV_PATH"
  # shellcheck disable=SC1090
  source "${ROOT_DIR}/scripts/ops/source_env.sh" "$ENV_PATH"
else
  echo "[warn] Env file not found at $ENV_PATH; ensure DATABASE_URL and other vars are exported in this shell"
fi

# 1) Bring up Console/API if needed (0.0.7 preflight)
if command -v docker >/dev/null 2>&1; then
  echo "[run] Bringing up Console/API via docker compose"
  bash "${ROOT_DIR}/scripts/ops/console_up.sh" --date "$DATE_TAG" || true
else
  echo "[warn] Docker not available; skipping compose bring-up. Assuming Console/API already running."
fi

# 2) Capture Console/API evidence (health/ready/transparency/apexmail) — 0.0.7
echo "[run] Capturing Console/API evidence"
SMTP_PASS="${SMTP_PASS:-}" \
bash "${ROOT_DIR}/scripts/ops/staging_console_capture.sh" \
  --base "${BASE_URL:-https://api.apexmediation.ee}" \
  --console "https://console.apexmediation.ee" \
  --date "$DATE_TAG"

# 3) Website tests (already done by helper if executed separately). Optional re-run.
if [ -d "${ROOT_DIR}/website" ]; then
  echo "[run] Website tests (security headers)"
  set +e
  npm --prefix "${ROOT_DIR}/website" run test | tee "${EVIDENCE_DIR}/website/security-tests.txt"
  set -e
fi

# 4) Billing & Stripe (0.0.9)
echo "[run] Billing staging helper"
bash "${ROOT_DIR}/scripts/ops/billing_staging.sh" "$DATE_TAG"

# 5) VRA exports (0.0.10)
echo "[run] VRA export helper"
DATABASE_URL="${DATABASE_URL:-}" bash "${ROOT_DIR}/scripts/ops/vra_export.sh" "$DATE_TAG"

# 6) Soak (0.0.12) — optional if k6 available
if command -v k6 >/dev/null 2>&1; then
  echo "[run] Soak test via k6 (BASE_URL=${BASE_URL:-unset})"
  BASE_URL="${BASE_URL:-https://api.apexmediation.ee}" VUS="${VUS:-5}" DURATION="${DURATION:-60m}" \
    bash "${ROOT_DIR}/scripts/ops/run_soak.sh" "$DATE_TAG"
else
  echo "[info] k6 not found; skipping soak. Set up k6 on this host to enable."
fi

# 7) Zip evidence (0.0.13)
echo "[run] Zipping evidence folder"
bash "${ROOT_DIR}/scripts/ops/zip_evidence.sh" "$DATE_TAG"

echo "[done] Staging orchestration complete. Evidence at ${EVIDENCE_DIR} and zip ${EVIDENCE_DIR}.zip"
