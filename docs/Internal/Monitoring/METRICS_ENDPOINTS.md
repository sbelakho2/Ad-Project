# Metrics Endpoints Documentation

**Last Updated:** January 2026  
**Owner:** Platform Team

---

## Overview

This document describes the Prometheus metrics endpoints exposed by Apex Mediation Platform services for monitoring and alerting.

---

## 1. Backend API Metrics

### Endpoint

```
GET /metrics
Host: api.apexmediation.ee (internal network only)
```

**Note:** The `/metrics` endpoint is not exposed publicly. It is accessible only from the internal network (Prometheus scraper).

### Implementation

**File:** `backend/src/utils/prometheus.ts`

```typescript
import { collectDefaultMetrics, register, Histogram, Counter, Gauge } from 'prom-client';

// Enable default Node.js metrics
collectDefaultMetrics();

// Custom metrics defined below...
```

### Available Metrics

#### 1.1 HTTP Request Metrics (RED)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `http_request_duration_seconds` | Histogram | method, route, status_code | Request latency (p50, p95, p99) |
| `http_requests_total` | Counter | method, route, status_code | Total request count |

**Buckets:** 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s

#### 1.2 RTB Auction Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `auction_latency_seconds` | Histogram | arm, exp_id | Auction decision latency |
| `rtb_adapter_latency_seconds` | Histogram | adapter | Per-adapter latency |
| `rtb_adapter_timeouts_total` | Counter | adapter | Adapter timeout count |
| `rtb_bid_requests_total` | Counter | adapter | Bid requests sent |
| `rtb_bid_responses_total` | Counter | adapter, status | Bid responses received |
| `rtb_no_fill_total` | Counter | adapter, reason | No-fill events |

#### 1.3 SSAI Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `ssai_sessions_created` | Counter | protocol, container | Session creation |
| `ssai_active_sessions` | Gauge | - | Current active sessions |
| `ssai_beacons_fired_total` | Counter | type | Beacon events |
| `ssai_beacon_errors_total` | Counter | type | Beacon failures |
| `ssai_decision_latency_seconds` | Histogram | - | Ad decision latency |
| `ssai_fill_rate` | Gauge | protocol | Current fill rate |

#### 1.4 Billing Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `adyen_payment_attempted_total` | Counter | - | Payment attempts |
| `adyen_payment_succeeded_total` | Counter | - | Successful payments |
| `adyen_payment_failed_total` | Counter | reason | Failed payments |
| `adyen_webhook_received_total` | Counter | event_type | Webhooks received |
| `adyen_webhook_failures_total` | Counter | reason | Webhook failures |

#### 1.5 Database Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `pg_pool_size` | Gauge | - | Connection pool size |
| `pg_pool_available` | Gauge | - | Available connections |
| `pg_pool_waiting` | Gauge | - | Waiting queries |
| `pg_query_duration_seconds` | Histogram | query | Query latency |

#### 1.6 Fraud Detection Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `fraud_score_calculated_total` | Counter | - | Scores calculated |
| `fraud_blocked_total` | Counter | reason | Blocked requests |
| `fraud_model_latency_seconds` | Histogram | - | ML model inference time |

#### 1.7 VRA (Transparency) Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `transparency_records_written` | Counter | - | Audit records written |
| `transparency_ingest_stage` | Gauge | stage | Pipeline stage tracking |
| `vra_verification_requests_total` | Counter | result | Verification attempts |

---

## 2. Console (Next.js) Metrics

### Endpoint

```
GET /api/metrics
Host: console.apexmediation.ee (internal network only)
```

### Available Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `nextjs_page_render_duration_seconds` | Histogram | Page render latency |
| `nextjs_api_route_duration_seconds` | Histogram | API route latency |
| `nextjs_build_info` | Gauge | Build metadata |

---

## 3. Default Node.js Metrics

When `PROMETHEUS_COLLECT_DEFAULTS=1`, these standard metrics are collected:

| Metric | Type | Description |
|--------|------|-------------|
| `process_cpu_user_seconds_total` | Counter | CPU time (user) |
| `process_cpu_system_seconds_total` | Counter | CPU time (system) |
| `process_resident_memory_bytes` | Gauge | Memory usage |
| `process_heap_bytes` | Gauge | Heap size |
| `nodejs_eventloop_lag_seconds` | Gauge | Event loop lag |
| `nodejs_active_handles_total` | Gauge | Active handles |
| `nodejs_active_requests_total` | Gauge | Active requests |
| `nodejs_gc_duration_seconds` | Histogram | GC pause duration |

---

## 4. Prometheus Scrape Configuration

**File:** `monitoring/prometheus.yml`

```yaml
scrape_configs:
  - job_name: 'backend-api'
    metrics_path: '/metrics'
    static_configs:
      - targets: ['${BACKEND_METRICS_TARGET}']
    relabel_configs:
      - target_label: service
        replacement: 'backend'

  - job_name: 'console'
    metrics_path: '/api/metrics'
    static_configs:
      - targets: ['${CONSOLE_METRICS_TARGET}']
```

---

## 5. Grafana Dashboards

Pre-built dashboards are available in `monitoring/grafana/`:

| Dashboard | Purpose | Key Panels |
|-----------|---------|------------|
| `api-red-metrics.json` | API health | Request rate, error rate, latency |
| `rtb-auction.json` | Auction performance | Adapter latency, fill rates |
| `ssai-sessions.json` | SSAI monitoring | Active sessions, beacon health |
| `billing-overview.json` | Payment health | Success rate, webhook status |

---

## 6. Alert Rules Based on Metrics

**File:** `monitoring/alerts.yml`

Key alerts derived from these metrics:

```yaml
# High error rate (from http_requests_total)
- alert: HighErrorRate
  expr: rate(http_requests_total{status_code=~"5.."}[5m]) 
        / rate(http_requests_total[5m]) > 0.05

# High latency (from http_request_duration_seconds)
- alert: HighLatency
  expr: histogram_quantile(0.95, 
        rate(http_request_duration_seconds_bucket[5m])) > 0.5

# Memory usage (from process_resident_memory_bytes)
- alert: HighMemoryUsage
  expr: process_resident_memory_bytes / 1024 / 1024 > 450
```

---

## 7. Custom Metric Registration

To add a new metric:

```typescript
// backend/src/utils/prometheus.ts

import { Counter, Histogram, Gauge } from 'prom-client';

// Counter example
export const myCounter = new Counter({
  name: 'my_operation_total',
  help: 'Total count of my operation',
  labelNames: ['status', 'type'] as const,
});

// Usage in service code
myCounter.labels({ status: 'success', type: 'batch' }).inc();
```

---

## 8. Security Considerations

1. **Network isolation:** Metrics endpoints are NOT exposed publicly
2. **No PII:** Metrics never include user identifiers or sensitive data
3. **Cardinality control:** Label values are bounded to prevent memory explosion
4. **Authentication:** Consider adding basic auth for metrics in multi-tenant setups

---

## 9. Troubleshooting

### Metrics not appearing

1. Check if service is running: `curl http://localhost:3000/health`
2. Check metrics endpoint: `curl http://localhost:3000/metrics`
3. Verify Prometheus target is up: Prometheus UI → Status → Targets

### High cardinality warning

If you see "high cardinality" warnings:
- Review labels for unbounded values (user IDs, UUIDs)
- Use bucketing or sampling for high-cardinality dimensions

### Metric collection disabled

Check environment variable:
```bash
echo $PROMETHEUS_COLLECT_DEFAULTS  # Should be "1"
```
