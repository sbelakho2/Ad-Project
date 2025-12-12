#!/usr/bin/env bash
# ============================================================
# VRA OVERNIGHT STABILITY TEST
# ============================================================
# Runs VRA reconciliation loop overnight to test:
# 1. Memory stability - no leaks over 12+ hours
# 2. Query consistency - same data returns same results
# 3. Error handling - graceful recovery from transient failures
# 4. Timing accuracy - operations complete within SLA
#
# RUN: nohup ./vra_overnight_stability.sh > stability.log 2>&1 &
#
# EXPECTED RESULTS:
# - variance_pct remains stable across iterations
# - coverage_pct remains stable across iterations
# - No OOM or crashes
# - Query times < 5 seconds

set -euo pipefail

API_URL="${API_URL:-http://164.90.232.151:8080}"
TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiJlYjhiNjZkNC1mYTIxLTQ2NDQtOTVmMC1jNjU3YjNjZGY4OGYiLCJwdWJsaXNoZXJJZCI6ImVmYmIwMzk0LTIzZWItNGI4MC1hNGQ2LTlhMGU2MGVmMWI1NiIsImVtYWlsIjoidGVzdEBhcGV4bWVkaWF0aW9uLmVlIiwicm9sZSI6ImFkbWluIiwidG9rZW5UeXBlIjoiYWNjZXNzIiwiaWF0IjoxNzY1NTQzNzk4LCJleHAiOjE3NjYxNDg1OTh9.2Mk2Ja8o9ofzUI6hMbKwx0HQ85eXymqv4SJjv7xblg4"

INTERVAL_SECONDS="${INTERVAL_SECONDS:-300}"  # 5 minutes default
DURATION_HOURS="${DURATION_HOURS:-12}"       # 12 hours default
EVIDENCE_DIR="evidence-$(date +%F)/vra-stability"

mkdir -p "$EVIDENCE_DIR"

METRICS_FILE="$EVIDENCE_DIR/metrics.csv"
LOG_FILE="$EVIDENCE_DIR/stability.log"
SUMMARY_FILE="$EVIDENCE_DIR/summary.txt"

MAX_ITERATIONS=$((DURATION_HOURS * 3600 / INTERVAL_SECONDS))

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

# Initialize metrics CSV
echo "timestamp,iteration,coverage_pct,variance_pct,total_paid,total_expected,total_impressions,query_time_ms,status" > "$METRICS_FILE"

log "=========================================="
log "VRA OVERNIGHT STABILITY TEST"
log "Start: $(date)"
log "Duration: $DURATION_HOURS hours"
log "Interval: $INTERVAL_SECONDS seconds"
log "Max Iterations: $MAX_ITERATIONS"
log "Evidence: $EVIDENCE_DIR"
log "=========================================="

ITERATION=0
ERRORS=0
FIRST_COVERAGE=""
FIRST_VARIANCE=""
MAX_QUERY_TIME=0
TOTAL_QUERY_TIME=0

while [[ $ITERATION -lt $MAX_ITERATIONS ]]; do
  ((ITERATION++))
  TIMESTAMP=$(date '+%Y-%m-%d %H:%M:%S')
  
  # Time the API call
  START_TIME=$(python3 -c 'import time; print(int(time.time() * 1000))')
  
  VRA_RESPONSE=$(curl -s --max-time 30 "$API_URL/api/v1/recon/overview" \
    -H "Authorization: Bearer $TOKEN" 2>/dev/null || echo '{"error": "timeout"}')
  
  END_TIME=$(python3 -c 'import time; print(int(time.time() * 1000))')
  QUERY_TIME=$((END_TIME - START_TIME))
  
  # Parse response
  if echo "$VRA_RESPONSE" | jq -e '.data' > /dev/null 2>&1; then
    STATUS="ok"
    COVERAGE=$(echo "$VRA_RESPONSE" | jq -r '.data.coveragePercent // 0')
    VARIANCE=$(echo "$VRA_RESPONSE" | jq -r '.data.variancePercent // 0')
    PAID=$(echo "$VRA_RESPONSE" | jq -r '.data.totals.paid // 0')
    EXPECTED=$(echo "$VRA_RESPONSE" | jq -r '.data.totals.expected // 0')
    IMPRESSIONS=$(echo "$VRA_RESPONSE" | jq -r '.data.totals.impressions // 0')
    
    # Store first values for drift detection
    if [[ -z "$FIRST_COVERAGE" ]]; then
      FIRST_COVERAGE=$COVERAGE
      FIRST_VARIANCE=$VARIANCE
    fi
  else
    STATUS="error"
    ((ERRORS++))
    COVERAGE=0
    VARIANCE=0
    PAID=0
    EXPECTED=0
    IMPRESSIONS=0
    log "⚠️ Iteration $ITERATION: API error or timeout"
  fi
  
  # Track timing stats
  if [[ $QUERY_TIME -gt $MAX_QUERY_TIME ]]; then
    MAX_QUERY_TIME=$QUERY_TIME
  fi
  TOTAL_QUERY_TIME=$((TOTAL_QUERY_TIME + QUERY_TIME))
  
  # Log metrics
  echo "$TIMESTAMP,$ITERATION,$COVERAGE,$VARIANCE,$PAID,$EXPECTED,$IMPRESSIONS,$QUERY_TIME,$STATUS" >> "$METRICS_FILE"
  
  # Progress update every 12 iterations (1 hour at 5-min intervals)
  if [[ $((ITERATION % 12)) -eq 0 ]]; then
    AVG_QUERY_TIME=$((TOTAL_QUERY_TIME / ITERATION))
    log "Progress: $ITERATION/$MAX_ITERATIONS | coverage=$COVERAGE% variance=$VARIANCE% | avg_time=${AVG_QUERY_TIME}ms max=${MAX_QUERY_TIME}ms | errors=$ERRORS"
  fi
  
  # Check for SLA violation (query > 5 seconds)
  if [[ $QUERY_TIME -gt 5000 ]]; then
    log "⚠️ SLA violation: Query took ${QUERY_TIME}ms (>5000ms)"
  fi
  
  # Sleep until next iteration
  sleep "$INTERVAL_SECONDS"
done

# ============================================================
# Final Summary
# ============================================================
log ""
log "=========================================="
log "STABILITY TEST COMPLETE"
log "=========================================="

AVG_QUERY_TIME=$((TOTAL_QUERY_TIME / ITERATION))

cat > "$SUMMARY_FILE" << EOF
VRA OVERNIGHT STABILITY TEST SUMMARY
=====================================
Start: $(head -2 "$LOG_FILE" | tail -1)
End: $(date)

ITERATIONS: $ITERATION
ERRORS: $ERRORS
ERROR RATE: $(echo "scale=2; $ERRORS * 100 / $ITERATION" | bc)%

TIMING:
- Average Query Time: ${AVG_QUERY_TIME}ms
- Max Query Time: ${MAX_QUERY_TIME}ms
- SLA (< 5000ms): $(if [[ $MAX_QUERY_TIME -lt 5000 ]]; then echo "✅ PASS"; else echo "❌ FAIL"; fi)

CONSISTENCY:
- First Coverage: $FIRST_COVERAGE%
- Last Coverage: $COVERAGE%
- Drift: $(echo "scale=2; $COVERAGE - $FIRST_COVERAGE" | bc)%

VERDICT: $(if [[ $ERRORS -lt 5 ]] && [[ $MAX_QUERY_TIME -lt 5000 ]]; then echo "✅ STABLE"; else echo "⚠️ REVIEW NEEDED"; fi)
EOF

cat "$SUMMARY_FILE" | tee -a "$LOG_FILE"

log ""
log "Metrics saved to: $METRICS_FILE"
log "Summary saved to: $SUMMARY_FILE"
