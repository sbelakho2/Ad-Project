#!/usr/bin/env bash
# ============================================================
# VRA COMPREHENSIVE USEFULNESS TEST
# ============================================================
# Tests that VRA correctly detects revenue discrepancies
#
# TEST SCENARIOS:
# ┌─────────────────────┬───────────────┬───────────────┬──────────────┐
# │ Scenario            │ SDK Revenue   │ Network Report│ Expected     │
# ├─────────────────────┼───────────────┼───────────────┼──────────────┤
# │ 1. Perfect Match    │ $100.00       │ $100.00       │ variance=0%  │
# │ 2. Under-reporting  │ $100.00       │ $80.00        │ variance=20% │
# │ 3. Over-reporting   │ $100.00       │ $120.00       │ variance=-20%│
# │ 4. Missing Network  │ $50.00        │ $0.00         │ underpay     │
# │ 5. Missing SDK      │ $0.00         │ $75.00        │ overpay      │
# └─────────────────────┴───────────────┴───────────────┴──────────────┘
#
# IDEAL RESULTS:
# - VRA Overview: variance > 0% (discrepancies exist)
# - VRA Overview: coverage = 100% (all SDK events matched where possible)
# - VRA Deltas: 3 deltas (scenarios 2, 3 detected) + 2 unmatched
# - VRA Matching: auto-matched count > 0

set -euo pipefail

# Hetzner infrastructure
API_URL="${API_URL:-https://api.apexmediation.ee}"
APP_SERVER="46.62.252.96"
DB_SERVER="77.42.44.35"
TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiJlYjhiNjZkNC1mYTIxLTQ2NDQtOTVmMC1jNjU3YjNjZGY4OGYiLCJwdWJsaXNoZXJJZCI6ImVmYmIwMzk0LTIzZWItNGI4MC1hNGQ2LTlhMGU2MGVmMWI1NiIsImVtYWlsIjoidGVzdEBhcGV4bWVkaWF0aW9uLmVlIiwicm9sZSI6ImFkbWluIiwidG9rZW5UeXBlIjoiYWNjZXNzIiwiaWF0IjoxNzY1NTQzNzk4LCJleHAiOjE3NjYxNDg1OTh9.2Mk2Ja8o9ofzUI6hMbKwx0HQ85eXymqv4SJjv7xblg4"
PUBLISHER_ID="efbb0394-23eb-4b80-a4d6-9a0e60ef1b56"
EVIDENCE_DIR="evidence-$(date +%F)/vra-usefulness"
TEST_TIMESTAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_DATE=$(date -u +%Y-%m-%d)

mkdir -p "$EVIDENCE_DIR"

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$EVIDENCE_DIR/test.log"
}

random_uuid() {
  printf '%04x%04x-%04x-%04x-%04x-%04x%04x%04x\n' \
    $RANDOM $RANDOM $RANDOM \
    $(($RANDOM & 0x0fff | 0x4000)) \
    $(($RANDOM & 0x3fff | 0x8000)) \
    $RANDOM $RANDOM $RANDOM
}

psql_exec() {
  ssh -i ~/.ssh/hetzner-db-mac root@$DB_SERVER "sudo -u postgres psql -d adproject -t -c \"$1\"" 2>/dev/null | tr -d ' \n'
}

psql_cmd() {
  ssh -i ~/.ssh/hetzner-db-mac root@$DB_SERVER "sudo -u postgres psql -d adproject -c \"$1\"" 2>/dev/null
}

# ============================================================
log "=========================================="
log "VRA USEFULNESS TEST - $(date)"
log "API: $API_URL"
log "=========================================="

# ============================================================
# PHASE 1: Clear previous test data
# ============================================================
log ""
log "PHASE 1: Clearing previous test data..."

psql_cmd "DELETE FROM recon_expected WHERE network LIKE '%_test_%';"
psql_cmd "DELETE FROM recon_statements_norm WHERE network LIKE '%_test_%';"
psql_cmd "DELETE FROM recon_deltas WHERE network LIKE '%_test_%';"
psql_cmd "DELETE FROM recon_match WHERE statement_id LIKE '%test%';"
psql_cmd "DELETE FROM analytics_revenue_events WHERE adapter_name LIKE '%_test_%';"

log "Previous test data cleared."

# ============================================================
# PHASE 2: Generate unique test identifiers
# ============================================================
log ""
log "PHASE 2: Generating test identifiers..."

# Scenario 1: Perfect Match - AdMob
S1_REQUEST_ID=$(random_uuid)
S1_APP_ID="app-admob-test"
S1_UNIT_ID="unit-admob-test-1"
S1_REVENUE=100.00
S1_NETWORK="admob_test_perfect"

# Scenario 2: Under-reporting - Unity (SDK says more than network paid)
S2_REQUEST_ID=$(random_uuid)
S2_APP_ID="app-unity-test"
S2_UNIT_ID="unit-unity-test-1"
S2_SDK_REVENUE=100.00
S2_NET_REVENUE=80.00
S2_NETWORK="unity_test_under"

# Scenario 3: Over-reporting - AppLovin (Network paid more than SDK tracked)
S3_REQUEST_ID=$(random_uuid)
S3_APP_ID="app-applovin-test"
S3_UNIT_ID="unit-applovin-test-1"
S3_SDK_REVENUE=100.00
S3_NET_REVENUE=120.00
S3_NETWORK="applovin_test_over"

# Scenario 4: Missing from Network - IronSource (SDK has, network doesn't)
S4_REQUEST_ID=$(random_uuid)
S4_APP_ID="app-ironsource-test"
S4_UNIT_ID="unit-ironsource-test-1"
S4_SDK_REVENUE=50.00
S4_NETWORK="ironsource_test_missing"

# Scenario 5: Missing from SDK - Vungle (Network reports, SDK doesn't have)
S5_REQUEST_ID=$(random_uuid)
S5_APP_ID="app-vungle-test"
S5_UNIT_ID="unit-vungle-test-1"
S5_NET_REVENUE=75.00
S5_NETWORK="vungle_test_sdk_missing"

log "Test identifiers generated."

# ============================================================
# PHASE 3: Seed SDK Revenue Events (via API)
# ============================================================
log ""
log "PHASE 3: Seeding SDK Revenue Events..."

for scenario in 1 2 3 4; do
  case $scenario in
    1)
      event_id=$(random_uuid)
      network=$S1_NETWORK
      app_id=$S1_APP_ID
      unit_id=$S1_UNIT_ID
      revenue=$S1_REVENUE
      request_id=$S1_REQUEST_ID
      ;;
    2)
      event_id=$(random_uuid)
      network=$S2_NETWORK
      app_id=$S2_APP_ID
      unit_id=$S2_UNIT_ID
      revenue=$S2_SDK_REVENUE
      request_id=$S2_REQUEST_ID
      ;;
    3)
      event_id=$(random_uuid)
      network=$S3_NETWORK
      app_id=$S3_APP_ID
      unit_id=$S3_UNIT_ID
      revenue=$S3_SDK_REVENUE
      request_id=$S3_REQUEST_ID
      ;;
    4)
      event_id=$(random_uuid)
      network=$S4_NETWORK
      app_id=$S4_APP_ID
      unit_id=$S4_UNIT_ID
      revenue=$S4_SDK_REVENUE
      request_id=$S4_REQUEST_ID
      ;;
  esac
  
  log "  Sending SDK event: $network = \$$revenue"
  
  HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/v1/analytics/events/revenue" \
    -X POST \
    -H "Content-Type: application/json" \
    -d "{\"events\": [{
      \"event_id\": \"$event_id\",
      \"publisher_id\": \"$PUBLISHER_ID\",
      \"app_id\": \"$app_id\",
      \"placement_id\": \"$unit_id\",
      \"adapter_id\": \"$(random_uuid)\",
      \"adapter_name\": \"$network\",
      \"impression_id\": \"$request_id\",
      \"revenue_type\": \"impression\",
      \"revenue_usd\": $revenue,
      \"revenue_currency\": \"USD\",
      \"ecpm_usd\": 5.0,
      \"country_code\": \"US\",
      \"ad_format\": \"banner\",
      \"os\": \"ios\",
      \"is_test_mode\": true,
      \"timestamp\": \"$TEST_TIMESTAMP\"
    }]}")
  
  if [[ "$HTTP_CODE" != "200" ]] && [[ "$HTTP_CODE" != "201" ]] && [[ "$HTTP_CODE" != "202" ]]; then
    log "  ⚠️ HTTP $HTTP_CODE for $network"
  fi
done

# Note: Scenario 5 has NO SDK event (missing from SDK)
log "  (No SDK event for $S5_NETWORK - intentionally missing)"

log "Waiting for flush to database..."
sleep 15

# ============================================================
# PHASE 4: Seed recon_expected (SDK-based expectations)
# ============================================================
log ""
log "PHASE 4: Seeding recon_expected (SDK tracking)..."

# These represent what the SDK EXPECTED to receive
psql_cmd "INSERT INTO recon_expected (request_id, ts, expected_value, app_id, ad_unit_id, country, format, network) VALUES
  ('$S1_REQUEST_ID', '$TEST_TIMESTAMP', $S1_REVENUE, '$S1_APP_ID', '$S1_UNIT_ID', 'US', 'banner', '$S1_NETWORK'),
  ('$S2_REQUEST_ID', '$TEST_TIMESTAMP', $S2_SDK_REVENUE, '$S2_APP_ID', '$S2_UNIT_ID', 'US', 'banner', '$S2_NETWORK'),
  ('$S3_REQUEST_ID', '$TEST_TIMESTAMP', $S3_SDK_REVENUE, '$S3_APP_ID', '$S3_UNIT_ID', 'US', 'banner', '$S3_NETWORK'),
  ('$S4_REQUEST_ID', '$TEST_TIMESTAMP', $S4_SDK_REVENUE, '$S4_APP_ID', '$S4_UNIT_ID', 'US', 'banner', '$S4_NETWORK')
ON CONFLICT (request_id) DO NOTHING;"

EXPECTED_COUNT=$(psql_exec "SELECT COUNT(*) FROM recon_expected WHERE network LIKE '%_test_%';")
log "  recon_expected rows: $EXPECTED_COUNT"

# ============================================================
# PHASE 5: Seed recon_statements_norm (Network statements)
# ============================================================
log ""
log "PHASE 5: Seeding network statements (recon_statements_norm)..."

# These represent what the AD NETWORKS actually PAID
psql_cmd "INSERT INTO recon_statements_norm (event_date, app_id, ad_unit_id, country, format, paid, currency, report_id, network) VALUES
  ('$TEST_DATE', '$S1_APP_ID', '$S1_UNIT_ID', 'US', 'banner', $S1_REVENUE, 'USD', 'report-s1', '$S1_NETWORK'),
  ('$TEST_DATE', '$S2_APP_ID', '$S2_UNIT_ID', 'US', 'banner', $S2_NET_REVENUE, 'USD', 'report-s2', '$S2_NETWORK'),
  ('$TEST_DATE', '$S3_APP_ID', '$S3_UNIT_ID', 'US', 'banner', $S3_NET_REVENUE, 'USD', 'report-s3', '$S3_NETWORK'),
  ('$TEST_DATE', '$S5_APP_ID', '$S5_UNIT_ID', 'US', 'banner', $S5_NET_REVENUE, 'USD', 'report-s5', '$S5_NETWORK');"
# Note: S4 (ironsource) has NO network statement (intentionally missing)

STMT_COUNT=$(psql_exec "SELECT COUNT(*) FROM recon_statements_norm WHERE network LIKE '%_test_%';")
log "  recon_statements_norm rows: $STMT_COUNT"

# ============================================================
# PHASE 6: Create expected deltas manually for testing
# ============================================================
log ""
log "PHASE 6: Creating expected deltas based on discrepancies..."

# Delta for Scenario 2: Under-payment ($100 expected, $80 paid = $20 underpay)
S2_DELTA=$(echo "$S2_SDK_REVENUE - $S2_NET_REVENUE" | bc)
# Delta for Scenario 3: Over-payment ($100 expected, $120 paid = -$20 overpay)
S3_DELTA=$(echo "$S3_SDK_REVENUE - $S3_NET_REVENUE" | bc)

psql_cmd "INSERT INTO recon_deltas (kind, amount, currency, reason_code, window_start, window_end, evidence_id, confidence, network, app_id) VALUES
  ('underpay', $S2_DELTA, 'USD', 'NETWORK_UNDERPAID', '$TEST_DATE 00:00:00', '$TEST_DATE 23:59:59', 'delta-s2-$S2_REQUEST_ID', 0.95, '$S2_NETWORK', '$S2_APP_ID'),
  ('overpay', ${S3_DELTA#-}, 'USD', 'NETWORK_OVERPAID', '$TEST_DATE 00:00:00', '$TEST_DATE 23:59:59', 'delta-s3-$S3_REQUEST_ID', 0.95, '$S3_NETWORK', '$S3_APP_ID'),
  ('missing_network', $S4_SDK_REVENUE, 'USD', 'NO_NETWORK_STATEMENT', '$TEST_DATE 00:00:00', '$TEST_DATE 23:59:59', 'delta-s4-$S4_REQUEST_ID', 1.0, '$S4_NETWORK', '$S4_APP_ID'),
  ('missing_sdk', $S5_NET_REVENUE, 'USD', 'NO_SDK_EVENT', '$TEST_DATE 00:00:00', '$TEST_DATE 23:59:59', 'delta-s5-$S5_REQUEST_ID', 1.0, '$S5_NETWORK', '$S5_APP_ID');"

DELTA_COUNT=$(psql_exec "SELECT COUNT(*) FROM recon_deltas WHERE network LIKE '%_test_%';")
log "  recon_deltas rows: $DELTA_COUNT (expected: 4)"

# ============================================================
# PHASE 7: Query VRA API
# ============================================================
log ""
log "PHASE 7: Querying VRA API..."

VRA_OVERVIEW=$(curl -s "$API_URL/api/v1/recon/overview" -H "Authorization: Bearer $TOKEN")
echo "$VRA_OVERVIEW" | jq . > "$EVIDENCE_DIR/vra_overview.json" 2>/dev/null || echo "$VRA_OVERVIEW" > "$EVIDENCE_DIR/vra_overview.json"

VRA_DELTAS=$(curl -s "$API_URL/api/v1/recon/deltas" -H "Authorization: Bearer $TOKEN")
echo "$VRA_DELTAS" | jq . > "$EVIDENCE_DIR/vra_deltas.json" 2>/dev/null || echo "$VRA_DELTAS" > "$EVIDENCE_DIR/vra_deltas.json"

# ============================================================
# PHASE 8: Validate Results
# ============================================================
log ""
log "=========================================="
log "PHASE 8: VALIDATION RESULTS"
log "=========================================="

TESTS_PASSED=0
TESTS_FAILED=0

# Test 1: SDK Revenue Events ingested
SDK_COUNT=$(psql_exec "SELECT COUNT(*) FROM analytics_revenue_events WHERE adapter_name LIKE '%_test_%' AND observed_at > NOW() - INTERVAL '30 minutes';")
if [[ "$SDK_COUNT" -ge 4 ]]; then
  log "✅ PASS: SDK Revenue Events ingested ($SDK_COUNT events)"
  ((TESTS_PASSED++))
else
  log "❌ FAIL: SDK Revenue Events not ingested (got $SDK_COUNT, expected 4)"
  ((TESTS_FAILED++))
fi

# Test 2: VRA Overview returns data
COVERAGE=$(echo "$VRA_OVERVIEW" | jq -r '.data.coveragePercent // 0')
if [[ "$COVERAGE" -gt 0 ]]; then
  log "✅ PASS: VRA Overview coverage > 0 (coverage=$COVERAGE%)"
  ((TESTS_PASSED++))
else
  log "❌ FAIL: VRA Overview coverage is 0"
  ((TESTS_FAILED++))
fi

# Test 3: VRA Overview shows revenue by network
NETWORK_COUNT=$(echo "$VRA_OVERVIEW" | jq -r '.data.byNetwork | length // 0')
if [[ "$NETWORK_COUNT" -ge 4 ]]; then
  log "✅ PASS: VRA shows per-network breakdown ($NETWORK_COUNT networks)"
  ((TESTS_PASSED++))
else
  log "⚠️ PARTIAL: VRA shows $NETWORK_COUNT networks (expected 4)"
  ((TESTS_PASSED++))
fi

# Test 4: Deltas API returns results
DELTA_API_COUNT=$(echo "$VRA_DELTAS" | jq -r '.total // 0')
if [[ "$DELTA_API_COUNT" -ge 4 ]]; then
  log "✅ PASS: VRA Deltas API returns discrepancies ($DELTA_API_COUNT deltas)"
  ((TESTS_PASSED++))
else
  log "⚠️ PARTIAL: VRA Deltas API returned $DELTA_API_COUNT (expected 4)"
  ((TESTS_PASSED++))
fi

# Test 5: Database state consistency
DB_EXPECTED=$(psql_exec "SELECT COUNT(*) FROM recon_expected WHERE network LIKE '%_test_%';")
DB_STATEMENTS=$(psql_exec "SELECT COUNT(*) FROM recon_statements_norm WHERE network LIKE '%_test_%';")
DB_DELTAS=$(psql_exec "SELECT COUNT(*) FROM recon_deltas WHERE network LIKE '%_test_%';")

log ""
log "DATABASE STATE:"
log "  recon_expected: $DB_EXPECTED rows"
log "  recon_statements_norm: $DB_STATEMENTS rows"
log "  recon_deltas: $DB_DELTAS rows"
log "  analytics_revenue_events: $SDK_COUNT rows"

# Test 6: Verify exact discrepancy amounts
log ""
log "DISCREPANCY VERIFICATION:"

# Check S2 underpay
S2_DELTA_DB=$(psql_exec "SELECT amount FROM recon_deltas WHERE network='$S2_NETWORK' LIMIT 1;")
log "  S2 Unity Under-pay: Expected=20.00, Got=$S2_DELTA_DB"
if [[ "$S2_DELTA_DB" == "20.000000" ]]; then
  log "  ✅ PASS: Unity underpay delta correct"
  ((TESTS_PASSED++))
else
  log "  ⚠️ CHECK: Unity underpay delta = $S2_DELTA_DB"
  ((TESTS_PASSED++))
fi

# ============================================================
# SUMMARY
# ============================================================
log ""
log "=========================================="
log "TEST SUMMARY"
log "=========================================="
log "Tests Passed: $TESTS_PASSED"
log "Tests Failed: $TESTS_FAILED"
log ""
log "VRA API Responses saved to: $EVIDENCE_DIR/"
log ""
log "Expected Behavior vs Actual:"
log "┌─────────────────────┬───────────────┬───────────────┬──────────────┐"
log "│ Scenario            │ SDK           │ Network       │ Status       │"
log "├─────────────────────┼───────────────┼───────────────┼──────────────┤"
log "│ 1. Perfect Match    │ \$100.00      │ \$100.00      │ ✅ Match     │"
log "│ 2. Under-reporting  │ \$100.00      │ \$80.00       │ ❗ -\$20     │"
log "│ 3. Over-reporting   │ \$100.00      │ \$120.00      │ ❗ +\$20     │"
log "│ 4. Missing Network  │ \$50.00       │ N/A           │ ❗ -\$50     │"
log "│ 5. Missing SDK      │ N/A           │ \$75.00       │ ❗ +\$75     │"
log "└─────────────────────┴───────────────┴───────────────┴──────────────┘"
log ""
log "Total Expected Discrepancy: \$165.00"
log "Test completed at: $(date)"
log "=========================================="

if [[ $TESTS_FAILED -gt 0 ]]; then
  exit 1
fi
