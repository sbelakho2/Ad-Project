#!/usr/bin/env bash
# Quick verification for continuous SLO population and website wiring
# Usage:
#   ./scripts/ops/slo_verify.sh
#   API_URL=https://api.apexmediation.ee ./scripts/ops/slo_verify.sh

set -euo pipefail

API_URL=${API_URL:-https://api.apexmediation.ee}
WEBSITE_URL=${WEBSITE_URL:-https://apexmediation.ee}

RED() { printf "\033[31m%s\033[0m\n" "$*"; }
GRN() { printf "\033[32m%s\033[0m\n" "$*"; }
YEL() { printf "\033[33m%s\033[0m\n" "$*"; }

fail() { RED "[FAIL] $*"; exit 1; }
ok() { GRN "[OK]   $*"; }
info() { YEL "[INFO] $*"; }

info "API_URL=$API_URL"
info "WEBSITE_URL=$WEBSITE_URL"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

info "Fetching status.json ..."
if ! curl -fsS "$API_URL/api/v1/status.json" -o "$tmpdir/status.json"; then
  fail "status.json not reachable"
fi

status_value=$(jq -r '.status // empty' "$tmpdir/status.json" 2>/dev/null || true)
if [[ -z "$status_value" ]]; then
  fail "status.json missing 'status' field"
fi
ok "status.json reachable (status=$status_value)"

info "Fetching slo.json ..."
if ! curl -fsS "$API_URL/api/v1/slo.json?days=30" -o "$tmpdir/slo.json"; then
  fail "slo.json not reachable"
fi

if ! jq -e '.slis' "$tmpdir/slo.json" >/dev/null 2>&1; then
  fail "slo.json missing 'slis' root object"
fi
ok "slo.json reachable and has SLIs"

info "Checking website routes ..."
code_slo=$(curl -s -o /dev/null -w "%{http_code}" "$WEBSITE_URL/slo")
if [[ "$code_slo" != "200" ]]; then
  fail "GET $WEBSITE_URL/slo returned $code_slo (expected 200)"
fi
ok "/slo returns 200"

code_status=$(curl -s -o /dev/null -w "%{http_code}" "$WEBSITE_URL/status")
if [[ "$code_status" != "200" ]]; then
  fail "GET $WEBSITE_URL/status returned $code_status (expected 200)"
fi
ok "/status returns 200"

echo ""
ok "Verification passed. Continuous SLO population and website wiring look healthy."
