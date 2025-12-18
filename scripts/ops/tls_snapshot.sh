#!/usr/bin/env bash
set -euo pipefail

# TLS Snapshot Helper
#
# Captures basic TLS/redirect/HSTS evidence into an evidence folder.
#
# Evidence files written:
#  - verify-redirects.txt
#  - verify-tls.txt
#  - verify-hsts.txt
#
# Usage:
#   BASE_URL=https://api.apexmediation.ee \
#   bash scripts/ops/tls_snapshot.sh 2025-12-19

DATE_TAG="${1:-$(date +%F)}"
EVIDENCE_DIR="evidence-${DATE_TAG}/infrastructure"
mkdir -p "${EVIDENCE_DIR}"

BASE_URL="${BASE_URL:-https://api.apexmediation.ee}"

# Redirects
{
  echo "[info] Checking redirects for ${BASE_URL}"
  curl -sSIL "${BASE_URL}" || true
} | tee "${EVIDENCE_DIR}/verify-redirects.txt" >/dev/null

# TLS details
{
  echo "[info] Checking TLS handshake for ${BASE_URL}"
  # Prefer openssl when available; fall back to curl output.
  host="${BASE_URL#https://}"
  host="${host%%/*}"
  if command -v openssl >/dev/null 2>&1; then
    echo | openssl s_client -servername "${host}" -connect "${host}:443" -showcerts 2>/dev/null | sed -n '1,160p' || true
  else
    curl -sSIL "${BASE_URL}" || true
  fi
} | tee "${EVIDENCE_DIR}/verify-tls.txt" >/dev/null

# HSTS header presence
{
  echo "[info] Checking HSTS header for ${BASE_URL}"
  curl -sSI "${BASE_URL}" | grep -i 'strict-transport-security' || echo "[warn] Strict-Transport-Security header not present"
} | tee "${EVIDENCE_DIR}/verify-hsts.txt" >/dev/null

echo "[done] TLS snapshots saved under ${EVIDENCE_DIR}"