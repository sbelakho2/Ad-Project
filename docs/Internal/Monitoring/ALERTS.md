# Prometheus Alerts Reference

> **Note:** For the current Hetzner production deployment alerts, see [docs/Monitoring/MONITORING_INFRASTRUCTURE.md](../../Monitoring/MONITORING_INFRASTRUCTURE.md). This document covers general alert patterns and runbooks.

Complete guide to Prometheus alert rules for the ApexMediation platform.

## Table of Contents

1. [Alert Overview](#alert-overview)
2. [Production Alerts (Hetzner)](#production-alerts-hetzner)
3. [Alert Groups](#alert-groups)
4. [Severity Levels](#severity-levels)
5. [RTB Auction Alerts](#rtb-auction-alerts)
6. [Analytics (Postgres) Alerts](#analytics-postgres-alerts)
7. [Queue Health Alerts](#queue-health-alerts)
8. [Testing Alerts](#testing-alerts)
9. [Alertmanager Configuration](#alertmanager-configuration)
10. [Runbooks](#runbooks)

---

## Alert Overview

Alerts are defined in Prometheus rule files and evaluated every 15 seconds (default).

### Alert File Structure

```yaml
groups:
  - name: group_name
    interval: 30s  # Override default eval interval
    rules:
      - alert: AlertName
        expr: PromQL expression
        for: 5m  # Duration before alert fires
        labels:
          severity: warning|critical
        annotations:
          summary: Brief description
          description: Detailed explanation with values
```

### Alert Lifecycle

1. **Pending:** Condition true but within `for` duration
2. **Firing:** Condition true for longer than `for` duration
3. **Resolved:** Condition returns to false

---

## Production Alerts (Hetzner)

**Location:** `infrastructure/production/monitoring/alerts/`

| File | Alert Count | Purpose |
|------|-------------|---------|
| `backend.yml` | 7 | API health, latency, error rates |
| `database.yml` | 5 | PostgreSQL connections, queries, replication |
| `billing.yml` | 4 | Payment processing, revenue tracking |
| `infrastructure.yml` | 5 | Container health, disk, memory |

**Access Alerts:** http://203.0.113.10:9093 (Alertmanager)

---

## Alert Groups

### Legacy Kubernetes Alert Summary

| Group | Alerts | Purpose | File Location |
|-------|--------|---------|---------------|
| rtb_auction | 7 | RTB performance and adapter health | monitoring/alerts.yml |
| analytics_postgres | 3 | Analytics pipeline (Postgres) issues | monitoring/alerts.yml |
| queue_health | 4 | Background job queue monitoring | monitoring/alerts.yml |

**Total Legacy Alerts:** 14

### RTB Auction (7 alerts)

Monitor real-time bidding auction performance and adapter reliability.

**Evaluation Interval:** 30s

### Analytics (Postgres) (3 alerts)

Monitor analytics event ingestion, Postgres staging buffers, and write parity.

**Evaluation Interval:** 30s

### Queue Health (4 alerts)

Monitor background job processing (payments, emails, reporting).

**Evaluation Interval:** 30s

---

## Severity Levels

### Warning (severity: warning)

**Characteristics:**
- Degraded performance but system still functional
- Action required within 1-4 hours
- Notification: Slack channel

**Examples:**
- Latency 20% above normal
- Error rate 1-5%
- Queue backlog growing slowly

**Response:**
- Acknowledge alert
- Investigate root cause during business hours
- Document in incident log

### Critical (severity: critical)

**Characteristics:**
- System outage or severe degradation
- Immediate action required (within 15 minutes)
- Notification: Slack + PagerDuty

**Examples:**
- Latency >2x SLA
- Error rate >5%
- Complete service failure

**Response:**
- Page on-call engineer
- Investigate immediately
- Follow runbook for mitigation
- Post-mortem required

---

## RTB Auction Alerts

### 1. AuctionLatencyP95Warning

**Severity:** Warning  
**Threshold:** p95 latency >200ms for 5 minutes  
**SLA Impact:** Approaching RTB timeout limits

**Query:**
```promql
histogram_quantile(0.95, 
  sum by (le) (rate(auction_latency_seconds_bucket[5m]))
) > 0.2
```

**Symptoms:**
- Slower bid responses
- Reduced win rate
- User experience degradation

**Possible Causes:**
- Slow adapter responses
- Database query slowness
- High CPU usage
- Network latency

**Mitigation:**
1. Check "RTB Overview" Grafana dashboard
2. Identify slow adapters in "Adapter Latency p95" panel
3. Consider disabling problematic adapters temporarily
4. Scale up backend replicas: `kubectl scale deployment/backend --replicas=5`

**Runbook:** [docs/runbooks/rtb-high-latency.md](../runbooks/rtb-high-latency.md)

---

### 2. AuctionLatencyP95Critical

**Severity:** Critical  
**Threshold:** p95 latency >500ms for 5 minutes  
**SLA Impact:** Violates RTB timeout SLA (typically 200-300ms)

**Query:**
```promql
histogram_quantile(0.95, 
  sum by (le) (rate(auction_latency_seconds_bucket[5m]))
) > 0.5
```

**Symptoms:**
- Auction timeouts
- Significant win rate drop
- Revenue loss

**Immediate Actions:**
1. Page on-call engineer
2. Check system health: CPU, memory, network
3. Disable slow adapters via feature flag
4. Scale up infrastructure immediately
5. Consider enabling auction caching

**Runbook:** [docs/runbooks/rtb-critical-latency.md](../runbooks/rtb-critical-latency.md)

---

### 3. AdapterTimeoutSpikeShort

**Severity:** Warning  
**Threshold:** Adapter timeout rate >0.1/s (10% of requests) for 5 minutes  
**Impact:** Reduced fill rate, potential revenue loss

**Query:**
```promql
sum by (adapter) (rate(rtb_adapter_timeouts_total[5m])) > 0.1
```

**Symptoms:**
- Specific adapter consistently timing out
- No-fill rate increasing

**Possible Causes:**
- Adapter service degradation
- Network connectivity issues
- Adapter capacity limits
- Our request rate exceeding adapter QPS limits

**Mitigation:**
1. Identify adapter from alert labels
2. Check adapter status page
3. Temporarily reduce traffic to adapter: `ADAPTER_TRAFFIC_PERCENT_<name>=50`
4. Contact adapter support if issue persists
5. Consider failover to backup adapter

---

### 4. AdapterTimeoutSpikeSustained

**Severity:** Critical  
**Threshold:** Adapter timeout rate >0.05/s (5%) for 1 hour  
**Impact:** Sustained revenue loss

**Query:**
```promql
sum by (adapter) (rate(rtb_adapter_timeouts_total[1h])) > 0.05
```

**Immediate Actions:**
1. Disable problematic adapter: `ADAPTER_ENABLED_<name>=false`
2. Redeploy backend to pick up config change
3. Verify no-fill rate returns to normal
4. Open support ticket with adapter
5. Monitor other adapters for compensation

---

### 5. HighNoFillRate

**Severity:** Warning  
**Threshold:** No-fill rate >50% for 10 minutes  
**Impact:** Lost revenue opportunities

**Query:**
```promql
sum(rate(rtb_no_fill_total[5m])) 
/ 
(sum(rate(rtb_wins_total[5m])) + sum(rate(rtb_no_fill_total[5m]))) 
> 0.5
```

**Symptoms:**
- More than half of auctions returning no winner
- Lower than expected revenue

**Possible Causes:**
- CPM floor prices too high
- Multiple adapter timeouts
- Low advertiser demand
- Targeting criteria too narrow

**Mitigation:**
1. Check "Wins by Adapter" panel - are adapters responding?
2. Review CPM floor settings
3. Verify adapter configuration
4. Check demand trends in Console

---

### 6. HTTPErrorBudgetBurnShort

**Severity:** Critical  
**Threshold:** HTTP 5xx error rate >5% over 5 minutes  
**Impact:** SLO violation

**Query:**
```promql
sum(rate(http_requests_total{status=~"5.."}[5m])) 
/ 
sum(rate(http_requests_total[5m])) 
> 0.05
```

**Symptoms:**
- Multiple 500/502/503 errors
- API endpoint failures
- Client complaints

**Immediate Actions:**
1. Check application logs: `kubectl logs -n production deployment/backend --tail=100`
2. Identify error pattern (specific endpoint?)
3. Check database connectivity
4. Verify external service availability
5. Consider rollback if recent deployment

---

### 7. HTTPErrorBudgetBurnLong

**Severity:** Warning  
**Threshold:** HTTP 5xx error rate >1% over 1 hour  
**Impact:** Sustained SLO violation

**Query:**
```promql
sum(rate(http_requests_total{status=~"5.."}[1h])) 
/ 
sum(rate(http_requests_total[1h])) 
> 0.01
```

**Mitigation:**
1. Review error trends over past hour
2. Identify most frequent error types
3. Check for resource exhaustion (memory, connections)
4. Validate recent deployments
5. Scale infrastructure if needed

---

## Analytics (Postgres) Alerts

### 1. AnalyticsWriteFailures

**Severity:** Critical  
**Threshold:** Write failure rate >10/s for 5 minutes  
**Impact:** Data loss, incomplete analytics

**Query:**
```promql
sum(rate(analytics_events_failed_total[5m])) > 10
```

**Symptoms:**
- Events never land in Postgres analytics tables
- Data gaps in dashboards / reporting APIs
- `analytics_events_failed_total` spikes for one or more kinds

**Possible Causes:**
- Postgres migration deployed with incompatible schema
- PgBouncer pool exhausted (transactions stuck in WAITING state)
- Long-running merge holding locks on staging partitions
- Network interruptions between ingest workers and Postgres

**Immediate Actions:**
1. Grafana → Tracking & Ingest dashboard: confirm failing `kind`.
2. Inspect backend logs for `analyticsEventsFailedTotal` details.
3. Check staging table counts: `SELECT kind, COUNT(*) FROM analytics_events_stage GROUP BY 1;`
4. Review `pg_stat_activity` for blocked `analytics_merge_*` queries.
5. Pause ingest if necessary via `ANALYTICS_INGEST_ENABLED=false`, remediate, then resume.

**Runbook:** [docs/runbooks/analytics-postgres.md](../runbooks/analytics-postgres.md)

---

### 2. AnalyticsBufferGrowth

**Severity:** Warning  
**Threshold:** Buffer size >5k rows for 10 minutes  
**Impact:** Rising staging pressure and merge latency

**Query:**
```promql
max(analytics_ingest_buffer_size) > 5000
```

**Symptoms:**
- `analytics_ingest_buffer_size` panel flatlines high
- Merge workers stay busy with little throughput
- Replica lag creeping upward

**Possible Causes:**
- Merge jobs slowed by VACUUM/autovacuum
- Primary Postgres under-provisioned (I/O waits)
- Worker concurrency reduced (queue workers crashed)

**Mitigation:**
1. Identify offending `kind` in Grafana (buffer chart).
2. Inspect merge workers/logs for errors.
3. Check Postgres for long-running analytics queries.
4. Increase worker replicas or reduce batch sizes until buffer drains.

**Runbook:** [docs/runbooks/analytics-postgres.md](../runbooks/analytics-postgres.md)

---

### 3. AnalyticsPipelineBroken

**Severity:** Critical  
**Threshold:** Queue lag growing >100 events/s for 10 minutes  
**Impact:** Complete analytics outage

**Query:**
```promql
sum(rate(analytics_events_enqueued_total[5m]))
-
sum(rate(analytics_events_written_total[5m]))
> 100
```

**Symptoms:**
- Events pile up in Redis/worker queue
- Tracking dashboards stop updating
- Postgres staging tables balloon

**Immediate Actions:**
1. Validate worker pods/processes are running.
2. Check Redis/queue depth metrics.
3. Confirm Postgres `/ready` probe shows low replica lag; if not, fail traffic back to primary temporarily.
4. Inspect `analytics_ingest_buffer_size` to ensure merges resume after remediation.
5. Document incident per operator checklist.

**Runbook:** [docs/runbooks/analytics-postgres.md](../runbooks/analytics-postgres.md)

---

## Queue Health Alerts

### 1. QueueBacklogGrowth

**Severity:** Warning  
**Threshold:** Queue depth growing >50 jobs/s for 10 minutes  
**Impact:** Processing delays, eventual timeout

**Query:**
```promql
deriv(sum by (queue) (queue_depth)[5m:]) > 50
```

**Symptoms:**
- Job processing falling behind
- Increasing queue depth
- Delayed payments/emails

**Possible Causes:**
- Worker processing rate too slow
- Job failures causing retries
- Sudden traffic spike
- Worker pod crashes

**Mitigation:**
1. Check worker logs for errors
2. Scale up workers for affected queue
3. Increase batch size in worker config
4. Investigate failing jobs: `redis-cli LRANGE failed:queue_name 0 10`

---

### 2. QueueDepthCritical

**Severity:** Critical  
**Threshold:** Queue depth >10,000 jobs for 5 minutes  
**Impact:** System overload, potential OOM

**Query:**
```promql
sum by (queue) (queue_depth) > 10000
```

**Immediate Actions:**
1. Identify queue from alert labels
2. Emergency scale workers: `kubectl scale deployment/worker-<queue> --replicas=20`
3. Temporarily stop enqueueing if queue >50K
4. Consider clearing old jobs if >100K
5. Check for job processing failures

---

### 3. QueueOldestJobStale

**Severity:** Warning  
**Threshold:** Oldest job age >15 minutes  
**Impact:** User-facing delays (payment confirmation, emails)

**Query:**
```promql
time() - queue_oldest_job_timestamp > 900
```

**Symptoms:**
- Users not receiving emails/notifications
- Payment processing delays
- Reporting data stale

**Mitigation:**
1. Check if workers are processing: `queue_jobs_completed_total` metric
2. Verify workers have capacity (not CPU/memory throttled)
3. Check for stuck jobs: `redis-cli LRANGE queue_name -10 -1`
4. Manually process oldest jobs if critical

---

### 4. HighQueueFailureRate

**Severity:** Critical  
**Threshold:** Failed job rate >10% for 10 minutes  
**Impact:** Jobs not completing, retries exhausting

**Query:**
```promql
sum(rate(queue_jobs_failed_total[5m])) 
/ 
(sum(rate(queue_jobs_completed_total[5m])) + sum(rate(queue_jobs_failed_total[5m]))) 
> 0.1
```

**Symptoms:**
- Jobs repeatedly failing
- Dead letter queue filling up
- User-facing features broken

**Immediate Actions:**
1. Check worker logs for error patterns
2. Identify failing job type
3. Fix root cause (database connection, external API, etc.)
4. Redeploy workers with fix
5. Replay failed jobs from dead letter queue

---

## Testing Alerts

### Simulate Alert Conditions

#### Local Testing (amtool)

```bash
# Install amtool
brew install alertmanager

# Check alert configuration syntax
promtool check rules monitoring/alerts.yml

# Test specific alert expression
promtool query instant http://localhost:9090 \
  'histogram_quantile(0.95, sum by (le) (rate(auction_latency_seconds_bucket[5m]))) > 0.2'
```

#### Simulate Alerts to Fire

Use the following procedures to trigger each alert for testing on-call response:

##### 1. AuctionLatencyP95Warning/Critical

**Simulate Method:** Add artificial latency to auction endpoint

```bash
# Add 300ms delay (triggers warning after 5 min)
kubectl set env deployment/backend AUCTION_DELAY_MS=300 -n staging

# Add 600ms delay (triggers critical)
kubectl set env deployment/backend AUCTION_DELAY_MS=600 -n staging

# Monitor alert: http://prometheus:9090/alerts
# Wait 5-6 minutes for alert to fire

# Remove delay when done
kubectl set env deployment/backend AUCTION_DELAY_MS- -n staging
```

**Verify:** Check Grafana "RTB Overview" dashboard → Auction Latency p95 panel

##### 2. AdapterTimeoutSpike

**Simulate Method:** Configure test adapter to timeout

```bash
# Set adapter timeout to 1ms (forces timeouts)
kubectl set env deployment/backend ADAPTER_TIMEOUT_TEST=1 -n staging
kubectl set env deployment/backend ADAPTER_ENABLED_TEST=true -n staging

# Generate load
kubectl run load-test --image=grafana/k6 --rm -it --restart=Never -- \
  run -e BASE_URL=http://backend:4000 /scripts/auction-load-test.js

# Wait 5 minutes, check http://prometheus:9090/alerts

# Cleanup
kubectl set env deployment/backend ADAPTER_TIMEOUT_TEST- -n staging
```

**Verify:** Check `rtb_adapter_timeouts_total{adapter="test"}` metric

##### 3. HTTPErrorBudgetBurn

**Simulate Method:** Force HTTP 500 errors

```bash
# Enable error injection (50% error rate)
kubectl set env deployment/backend ERROR_INJECTION_RATE=0.5 -n staging

# Generate traffic
for i in {1..1000}; do
  curl -X POST http://backend:4000/api/v1/rtb/bid \
    -H "Content-Type: application/json" \
    -d '{"placementId":"test"}' &
done

# Wait 5 minutes
# Disable injection
kubectl set env deployment/backend ERROR_INJECTION_RATE- -n staging
```

**Verify:** Check `http_requests_total{status="500"}` in Prometheus

##### 4. AnalyticsWriteFailures

**Simulate Method:** Temporarily block Postgres analytics connectivity

```bash
# Block Postgres port temporarily (requires network policy)
kubectl apply -f - <<EOF
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: block-analytics-postgres
  namespace: staging
spec:
  podSelector:
    matchLabels:
      app: analytics-worker
  policyTypes:
  - Egress
  egress:
  - to:
    - podSelector:
        matchLabels:
          app: postgres
    ports:
    - protocol: TCP
      port: 5432
EOF

# Wait 5 minutes for alert
# Remove policy
kubectl delete networkpolicy block-analytics-postgres -n staging
```

**Verify:** Check `analytics_events_failed_total` metric

##### 5. QueueDepthCritical

**Simulate Method:** Enqueue jobs faster than processing

```bash
# Stop queue workers
kubectl scale deployment/queue-worker --replicas=0 -n staging

# Enqueue 15,000 jobs
for i in {1..15000}; do
  redis-cli LPUSH "queue:payment-processing" \
    '{"job":"test","id":"'$i'"}'
done

# Wait 5 minutes for alert
# Resume workers
kubectl scale deployment/queue-worker --replicas=3 -n staging
```

**Verify:** Check `queue_depth` metric or `redis-cli LLEN queue:payment-processing`

##### 6. AnalyticsPipelineBroken

**Simulate Method:** Pause analytics workers while continuing to enqueue events

```bash
# Scale analytics workers to zero so enqueued events pile up
kubectl scale deployment/analytics-worker --replicas=0 -n staging

# Generate analytics events
kubectl run load-test --image=curlimages/curl --rm -it --restart=Never -- \
  sh -c 'for i in $(seq 1 10000); do curl -s -X POST \
    http://backend:4000/api/v1/analytics/event \
    -H "Content-Type: application/json" \
    -d "{\"kind\":\"impression\",\"placementId\":\"test\"}"; sleep 0.1; done'

# Wait 10 minutes for queue lag to grow

# Restore workers
kubectl scale deployment/analytics-worker --replicas=3 -n staging
```

**Verify:** Check queue lag: `redis_queue_lag_seconds` metric

##### 7. HighQueueFailureRate

**Simulate Method:** Inject job processing failures

```bash
# Make jobs fail 20% of the time
kubectl set env deployment/queue-worker JOB_FAILURE_RATE=0.2 -n staging

# Enqueue jobs
for i in {1..1000}; do
  redis-cli LPUSH "queue:test" '{"job":"test","id":"'$i'"}'
done

# Wait 10 minutes
# Remove injection
kubectl set env deployment/queue-worker JOB_FAILURE_RATE- -n staging
```

**Verify:** Check `queue_jobs_failed_total` / `queue_jobs_completed_total` ratio

---

### Alert Simulation Best Practices

1. **Always use staging environment** for simulations (never production)
2. **Document what you're doing:** Post in Slack before starting
3. **Set a reminder** to clean up after simulation
4. **Verify alert fires** in Alertmanager: http://alertmanager:9093/#/alerts
5. **Test notification routing** (check Slack/PagerDuty receives alert)
6. **Follow runbook** as if it were a real incident
7. **Time your response** and document any runbook improvements

### Simulation Schedule

Run simulations quarterly to ensure:
- Runbooks are up-to-date
- On-call rotation is trained
- Alerting infrastructure works
- Notification channels function

**Q1 2025 Simulation:** February 15  
**Q2 2025 Simulation:** May 15  
**Q3 2025 Simulation:** August 15  
**Q4 2025 Simulation:** November 15

#### Unit Test Alerts

```yaml
# tests/alerts_test.yml
rule_files:
  - ../monitoring/alerts.yml

evaluation_interval: 1m

tests:
  - interval: 1m
    input_series:
      - series: 'auction_latency_seconds_bucket{le="0.1"}'
        values: '0+10x10'  # Increment by 10 every minute
      - series: 'auction_latency_seconds_bucket{le="0.5"}'
        values: '0+50x10'
      - series: 'auction_latency_seconds_bucket{le="+Inf"}'
        values: '0+100x10'

    alert_rule_test:
      - eval_time: 5m
        alertname: AuctionLatencyP95Warning
        exp_alerts:
          - exp_labels:
              severity: warning
            exp_annotations:
              summary: "Auction latency p95 >200ms"
```

Run tests:
```bash
promtool test rules tests/alerts_test.yml
```

---

## Alertmanager Configuration

### Routing by Severity

```yaml
# monitoring/alertmanager.yml
route:
  receiver: 'slack-warnings'
  group_by: ['alertname', 'cluster']
  group_wait: 10s
  group_interval: 5m
  repeat_interval: 4h
  
  routes:
    # Critical alerts go to PagerDuty + Slack
    - match:
        severity: critical
      receiver: 'pagerduty-critical'
      repeat_interval: 5m
      continue: true  # Also send to Slack
    
    # Warning alerts to Slack only
    - match:
        severity: warning
      receiver: 'slack-warnings'
      repeat_interval: 1h

receivers:
  - name: 'pagerduty-critical'
    pagerduty_configs:
      - service_key: '<PD_SERVICE_KEY>'
        description: '{{ .GroupLabels.alertname }}: {{ .CommonAnnotations.summary }}'
  
  - name: 'slack-warnings'
    slack_configs:
      - api_url: '<SLACK_WEBHOOK_URL>'
        channel: '#alerts-production'
        title: '{{ .GroupLabels.alertname }}'
        text: '{{ .CommonAnnotations.description }}'
```

### Silencing Alerts

Temporarily silence alerts during maintenance:

```bash
# Silence all RTB alerts for 1 hour
amtool silence add \
  alertname=~"Auction.*|Adapter.*" \
  --duration=1h \
  --comment="Maintenance: deploying new adapter config" \
  --author="ops@rival.com"

# List active silences
amtool silence query

# Expire silence early
amtool silence expire <silence-id>
```

### Inhibition Rules

Suppress redundant alerts:

```yaml
inhibit_rules:
  # If auction latency is critical, don't alert on warning
  - source_match:
      alertname: 'AuctionLatencyP95Critical'
    target_match:
      alertname: 'AuctionLatencyP95Warning'
    equal: ['cluster']
  
  # If the pipeline is in a critical state, suppress secondary analytics warnings
  - source_match:
      alertname: 'AnalyticsPipelineBroken'
    target_match_re:
      alertname: 'AnalyticsBuffer.*'
    equal: ['cluster']
```

---

## Runbooks

Detailed troubleshooting guides for each alert:

| Alert | Runbook | Priority |
|-------|---------|----------|
| AuctionLatencyP95Critical | [rtb-critical-latency.md](../runbooks/rtb-critical-latency.md) | P1 |
| HTTPErrorBudgetBurn | [http-error-spike.md](../runbooks/http-error-spike.md) | P1 |
| AnalyticsWriteFailures | [analytics-postgres.md](../runbooks/analytics-postgres.md) | P1 |
| AnalyticsPipelineBroken | [analytics-postgres.md](../runbooks/analytics-postgres.md) | P1 |
| AnalyticsBufferGrowth | [analytics-postgres.md](../runbooks/analytics-postgres.md) | P2 |
| QueueDepthCritical | [queue-backlog.md](../runbooks/queue-backlog.md) | P2 |
| AdapterTimeoutSpike | [adapter-timeouts.md](../runbooks/adapter-timeouts.md) | P2 |

Runbook template structure:
1. **Symptoms** - What users/systems experience
2. **Diagnosis** - How to confirm root cause
3. **Mitigation** - Immediate steps to restore service
4. **Resolution** - Long-term fix
5. **Prevention** - How to avoid recurrence

---

## Related Documentation

- [Grafana Dashboards](./GRAFANA_DASHBOARDS.md)
- [Monitoring Setup](../../monitoring/README.md)
- [Runbook Index](../runbooks/)
- [On-Call Guide](../Internal/ON_CALL.md)
