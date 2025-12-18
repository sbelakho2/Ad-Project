# Grafana Dashboards Reference

> **Note:** For the current Hetzner production deployment, see [docs/Monitoring/MONITORING_INFRASTRUCTURE.md](../../Monitoring/MONITORING_INFRASTRUCTURE.md). This document covers the general dashboard patterns and PromQL queries.

Complete guide to all Grafana dashboards for the ApexMediation platform.

## Table of Contents

1. [Dashboard Overview](#dashboard-overview)
2. [API RED Metrics](#api-red-metrics)
3. [RTB Overview](#rtb-overview)
4. [Tracking & Ingest](#tracking--ingest)
5. [Database & Queue](#database--queue)
6. [Dashboard Customization](#dashboard-customization)
7. [Common Patterns](#common-patterns)

---

## Dashboard Overview

### Production Dashboards (Hetzner)

| Dashboard | ID | Purpose |
|-----------|-----|---------|
| Executive Overview | `apex-executive-overview` | Platform health, KPIs, business metrics |
| RTB Analytics | `apex-rtb-analytics` | Fill rates, eCPM, adapter performance |
| Billing & Finance | `apex-billing-finance` | MRR/ARR, churn, payment processing |
| Database Performance | `apex-database` | PostgreSQL monitoring |

**Access:** http://46.62.252.96:3001 (Credentials stored in secret store)

### Legacy Kubernetes Dashboards

| Dashboard | Purpose | Key Metrics | Refresh Interval |
|-----------|---------|-------------|------------------|
| API RED Metrics | HTTP request monitoring (Rate, Errors, Duration) | Request rate, error %, p50/p95/p99 latency | 10s |
| RTB Overview | Real-time bidding auction performance | Auction latency, win rate, no-fill rate, adapter timeouts | 30s |
| Tracking & Ingest | Analytics event pipeline health | Events enqueued/written/failed, queue lag | 30s |
| DB & Queue | Database and job queue performance | Query latency, connection pool, queue depth, backlog age | 30s |

### Access Dashboards

**Development:**
```bash
# Port-forward Grafana
kubectl port-forward -n monitoring svc/grafana 3000:80

# Open in browser
open http://localhost:3000
# Username: admin
# Password: (from secret)
```

**Production (Hetzner):**
```
http://46.62.252.96:3001
```

---

## API RED Metrics

**Dashboard ID:** `api-red-metrics`  
**Panels:** 12

### Purpose

Monitor HTTP request Rate, Errors, and Duration across all API endpoints.

### Key Metrics

#### Request Rate (Requests/sec)

```promql
sum(rate(http_requests_total[5m]))
```

Shows total API throughput. Normal range: 100-5000 req/s depending on traffic.

#### Error Rate (%)

```promql
sum(rate(http_requests_total{status=~"5.."}[5m])) 
/ 
sum(rate(http_requests_total[5m])) * 100
```

**Thresholds:**
- Green: <1%
- Yellow: 1-5%
- Red: >5%

#### Latency Percentiles

```promql
# p50
histogram_quantile(0.5, 
  sum by (le) (rate(http_request_duration_seconds_bucket[5m]))
)

# p95
histogram_quantile(0.95, 
  sum by (le) (rate(http_request_duration_seconds_bucket[5m]))
)

# p99
histogram_quantile(0.99, 
  sum by (le) (rate(http_request_duration_seconds_bucket[5m]))
)
```

**Expected Values:**
- p50: <50ms
- p95: <200ms
- p99: <500ms

### Dashboard Screenshot

```
┌─────────────────────────────────────────────────────────────────────┐
│ API RED Metrics Dashboard                    [Last 1h] [↻ 10s]      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  Total Req/s    Error Rate     p50 Latency    p95 Latency           │
│  ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐          │
│  │ 2,450   │   │  0.8%    │   │  42ms    │   │  185ms   │          │
│  │ ↑ +12%  │   │  🟢      │   │  🟢      │   │  🟢      │          │
│  └─────────┘   └──────────┘   └──────────┘   └──────────┘          │
│                                                                       │
│  Request Rate by Endpoint                                            │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                        /api/v1/rtb/bid ▓▓▓▓▓▓▓▓▓▓▓            │  │
│  │                  /api/v1/analytics/event ▓▓▓▓▓                │  │
│  │                      /api/v1/users/me ▓▓▓                     │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Latency Percentiles (ms)                                            │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ 500│                                        ╱─ p99              │  │
│  │    │                                  ╱────╯                   │  │
│  │ 200│                           ╱─────╯ p95                     │  │
│  │    │                    ╱──────╯                               │  │
│  │  50│            ────────╯ p50                                  │  │
│  │   0└──────────────────────────────────────────────────────────┘  │
│      10:00   10:15   10:30   10:45   11:00                          │
│                                                                       │
│  Slowest Endpoints (Top 10)                                          │
│  ┌────────────────────────────────────────────┬──────┬──────────┐   │
│  │ Endpoint                                   │ p95  │ Req/s    │   │
│  ├────────────────────────────────────────────┼──────┼──────────┤   │
│  │ POST /api/v1/billing/invoices/:id/pdf     │ 892ms│ 12       │   │
│  │ GET  /api/v1/transparency/metrics          │ 445ms│ 8        │   │
│  │ POST /api/v1/rtb/bid                       │ 156ms│ 1,850    │   │
│  └────────────────────────────────────────────┴──────┴──────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### Panels

1. **Total Request Rate** (Stat) - Current req/s
   ```promql
   sum(rate(http_requests_total[5m]))
   ```

2. **Error Rate %** (Stat) - Current error percentage with color thresholds
   ```promql
   sum(rate(http_requests_total{status=~"5.."}[5m])) 
   / sum(rate(http_requests_total[5m])) * 100
   ```

3. **p50 Latency** (Stat) - Median response time
   ```promql
   histogram_quantile(0.5, 
     sum by (le) (rate(http_request_duration_seconds_bucket[5m]))
   )
   ```

4. **p95 Latency** (Stat) - 95th percentile response time
   ```promql
   histogram_quantile(0.95, 
     sum by (le) (rate(http_request_duration_seconds_bucket[5m]))
   )
   ```

5. **Request Rate by Endpoint** (Timeseries) - Top 10 endpoints
   ```promql
   topk(10, 
     sum by (route) (rate(http_requests_total[5m]))
   )
   ```

6. **Error Rate by Endpoint** (Timeseries) - Endpoints with errors
   ```promql
   sum by (route) (rate(http_requests_total{status=~"5.."}[5m]))
   > 0
   ```

7. **Latency Percentiles** (Timeseries) - p50/p95/p99 over time
   ```promql
   # p50 (green)
   histogram_quantile(0.50, sum by (le) (rate(http_request_duration_seconds_bucket[5m])))
   # p95 (yellow)
   histogram_quantile(0.95, sum by (le) (rate(http_request_duration_seconds_bucket[5m])))
   # p99 (red)
   histogram_quantile(0.99, sum by (le) (rate(http_request_duration_seconds_bucket[5m])))
   ```

8. **Status Code Distribution** (Timeseries) - 2xx/3xx/4xx/5xx counts
   ```promql
   sum by (status) (rate(http_requests_total[5m]))
   ```

9. **Slowest Endpoints** (Table) - Endpoints sorted by p95 latency
   ```promql
   # p95 latency
   histogram_quantile(0.95, 
     sum by (route, le) (rate(http_request_duration_seconds_bucket[5m]))
   )
   # Request rate
   sum by (route) (rate(http_requests_total[5m]))
   ```

10. **Error Distribution** (Table) - Error codes and counts
    ```promql
    sum by (status, route) (increase(http_requests_total{status=~"5.."}[1h]))
    > 0
    ```

11. **Request Size** (Timeseries) - Avg request body size
    ```promql
    avg(http_request_size_bytes)
    ```

12. **Response Size** (Timeseries) - Avg response body size
    ```promql
    avg(http_response_size_bytes)
    ```

### Usage

**Identify Slow Endpoints:**
1. Check "Slowest Endpoints" table
2. Click endpoint name to filter other panels
3. Review "Latency Percentiles" for trends

**Investigate Error Spikes:**
1. Check "Error Rate by Endpoint" timeseries
2. Note time of spike
3. Cross-reference with logs: `kubectl logs -n production deployment/backend --since=10m | grep ERROR`

---

## RTB Overview

**Dashboard ID:** `rtb-overview`  
**Panels:** 11  
**File:** `monitoring/grafana/rtb-overview.json`

### Purpose

Monitor real-time bidding auction performance, adapter health, and win/no-fill rates.

### Key Metrics

#### Auction Latency

```promql
# p50
histogram_quantile(0.5, 
  sum by (le) (rate(auction_latency_seconds_bucket[5m]))
)

# p95 (Critical for SLA)
histogram_quantile(0.95, 
  sum by (le) (rate(auction_latency_seconds_bucket[5m]))
)
```

**SLA Targets:**
- p50: <100ms (Green <100ms, Yellow 100-150ms, Red >150ms)
- p95: <200ms (Green <200ms, Yellow 200-300ms, Red >300ms)

#### Win Rate

```promql
sum(rate(rtb_wins_total[5m]))
```

Shows successful bid wins per second. Combine with total bid requests to calculate win %.

#### No-Fill Rate

```promql
sum(rate(rtb_no_fill_total[5m])) 
/ 
(sum(rate(rtb_wins_total[5m])) + sum(rate(rtb_no_fill_total[5m]))) * 100
```

**Thresholds:**
- Green: <30%
- Yellow: 30-50%
- Red: >50%

High no-fill indicates adapter issues or low CPM floors.

#### Adapter Timeouts

```promql
sum by (adapter) (rate(rtb_adapter_timeouts_total[5m]))
```

Timeouts per adapter per second. Should be <0.01/s (<1%) for healthy adapters.

### Dashboard Screenshot

```
┌─────────────────────────────────────────────────────────────────────┐
│ RTB Overview Dashboard                       [Last 1h] [↻ 30s]      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  Auction p50    Auction p95    Total Wins     No-Fill Rate          │
│  ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐          │
│  │  78ms   │   │  142ms   │   │  1,245/s │   │  32%     │          │
│  │  🟢     │   │  🟢      │   │  ↑ +8%   │   │  🟢      │          │
│  └─────────┘   └──────────┘   └──────────┘   └──────────┘          │
│                                                                       │
│  Wins by Adapter                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │            AdMob ████████████████████████████                  │  │
│  │     Unity Ads ██████████████                                   │  │
│  │   AppLovin ████████                                            │  │
│  │   ironSource ████                                              │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Adapter Performance (Last 1h)                                       │
│  ┌────────────┬─────────┬────────┬──────────┬────────┬────────┐    │
│  │ Adapter    │ Wins    │ p95 ms │ Timeouts │ Win %  │ Status │    │
│  ├────────────┼─────────┼────────┼──────────┼────────┼────────┤    │
│  │ AdMob      │ 45,230  │ 98ms   │ 12       │ 42%    │ 🟢     │    │
│  │ Unity Ads  │ 28,156  │ 125ms  │ 8        │ 26%    │ 🟢     │    │
│  │ AppLovin   │ 15,890  │ 156ms  │ 45       │ 15%    │ 🟡     │    │
│  │ ironSource │ 8,234   │ 210ms  │ 156      │ 8%     │ 🔴     │    │
│  └────────────┴─────────┴────────┴──────────┴────────┴────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

### Panels

1. **Auction Latency p50** (Stat) - Median auction time
   ```promql
   histogram_quantile(0.5, 
     sum by (le) (rate(auction_latency_seconds_bucket[5m]))
   )
   ```

2. **Auction Latency p95** (Stat) - 95th percentile with color thresholds
   ```promql
   histogram_quantile(0.95, 
     sum by (le) (rate(auction_latency_seconds_bucket[5m]))
   )
   ```
   **Thresholds:** Green <200ms, Yellow 200-300ms, Red >300ms

3. **Total Wins** (Stat) - Successful auctions per second
   ```promql
   sum(rate(rtb_wins_total[5m]))
   ```

4. **No-Fill Rate** (Stat) - Percentage of auctions without winner
   ```promql
   sum(rate(rtb_no_fill_total[5m])) 
   / (sum(rate(rtb_wins_total[5m])) + sum(rate(rtb_no_fill_total[5m]))) 
   * 100
   ```
   **Thresholds:** Green <30%, Yellow 30-50%, Red >50%

5. **Auction Latency Percentiles** (Timeseries) - p50/p75/p95/p99 over time
   ```promql
   # p50
   histogram_quantile(0.50, sum by (le) (rate(auction_latency_seconds_bucket[5m])))
   # p75
   histogram_quantile(0.75, sum by (le) (rate(auction_latency_seconds_bucket[5m])))
   # p95
   histogram_quantile(0.95, sum by (le) (rate(auction_latency_seconds_bucket[5m])))
   # p99
   histogram_quantile(0.99, sum by (le) (rate(auction_latency_seconds_bucket[5m])))
   ```

6. **Wins by Adapter** (Timeseries) - Stacked area chart showing which adapters win
   ```promql
   sum by (adapter) (rate(rtb_wins_total[5m]))
   ```

7. **No-Fill Rate Trend** (Timeseries) - No-fill percentage with 50% threshold line
   ```promql
   sum(rate(rtb_no_fill_total[5m])) 
   / (sum(rate(rtb_wins_total[5m])) + sum(rate(rtb_no_fill_total[5m]))) 
   * 100
   ```

8. **Adapter Latency p95** (Timeseries) - Per-adapter response times
   ```promql
   histogram_quantile(0.95, 
     sum by (adapter, le) (rate(rtb_adapter_latency_seconds_bucket[5m]))
   )
   ```

9. **Adapter Timeouts** (Timeseries) - Timeout counts by adapter
   ```promql
   sum by (adapter) (rate(rtb_adapter_timeouts_total[5m]))
   ```

10. **Adapter Performance Summary** (Table) - 1h aggregation showing latency, wins, timeouts, win %
    ```promql
    # Wins (last 1h)
    sum by (adapter) (increase(rtb_wins_total[1h]))
    
    # p95 latency
    histogram_quantile(0.95, 
      sum by (adapter, le) (rate(rtb_adapter_latency_seconds_bucket[1h]))
    )
    
    # Timeouts (last 1h)
    sum by (adapter) (increase(rtb_adapter_timeouts_total[1h]))
    
    # Win percentage
    sum by (adapter) (increase(rtb_wins_total[1h]))
    / (sum by (adapter) (increase(rtb_wins_total[1h])) 
       + sum by (adapter) (increase(rtb_no_fill_total[1h])))
    * 100
    ```

11. **Auction Volume** (Timeseries) - Total auction requests per second
    ```promql
    sum(rate(auction_latency_seconds_count[5m]))
    ```

### Usage

**Diagnose Slow Auctions:**
1. Check "Auction Latency p95" stat - if red, investigate
2. Review "Adapter Latency p95" timeseries to identify slow adapter
3. Check "Adapter Timeouts" - high timeouts indicate adapter issues
4. Action: Consider disabling problematic adapter or increasing timeout

**Optimize Win Rate:**
1. Check "Wins by Adapter" - identify which adapters contribute most
2. Review "Adapter Performance Summary" table - sort by win %
3. Compare latency vs wins - fast adapters with low wins may need CPM adjustment
4. Cross-reference with "No-Fill Rate" - high no-fill suggests CPM floors too high

---

## Tracking & Ingest

**Dashboard ID:** `tracking-ingest`  
**Panels:** 10  
**File:** `monitoring/grafana/tracking-ingest.json`

### Purpose

Monitor analytics event ingestion pipeline from SDK → Redis queue → Postgres analytics tables.

### Key Metrics

#### Events Enqueued

```promql
sum(rate(analytics_events_enqueued_total[5m]))
```

Events added to Redis queue per second. Should match SDK impression/click volume.

#### Events Written (Postgres)

```promql
sum(rate(analytics_events_written_total[5m]))
```

Successful INSERT + merge cycles into Postgres analytics tables. Should be close to enqueued rate (within 5%).

#### Events Failed

```promql
sum(rate(analytics_events_failed_total[5m]))
```

Failed writes due to validation, schema errors, or Postgres issues. Should be <1% of total.

#### Success Rate

```promql
sum(rate(analytics_events_written_total[5m])) 
/ 
(sum(rate(analytics_events_written_total[5m])) + sum(rate(analytics_events_failed_total[5m]))) * 100
```

**Thresholds:**
- Green: >99%
- Yellow: 95-99%
- Red: <95%

#### Queue Lag

```promql
redis_queue_lag_seconds
```

Time difference between oldest event in queue and now. High lag (>60s) indicates processing backlog.

### Dashboard Screenshot

```
┌─────────────────────────────────────────────────────────────────────┐
│ Tracking & Ingest Dashboard                  [Last 1h] [↻ 30s]      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  Enqueued/s     Written/s      Failed/s       Success Rate          │
│  ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐          │
│  │  3,450  │   │  3,420   │   │    8     │   │  99.8%   │          │
│  │  ↑ +5%  │   │  ↑ +5%   │   │  🟢      │   │  🟢      │          │
│  └─────────┘   └──────────┘   └──────────┘   └──────────┘          │
│                                                                       │
│  Events by Kind (Written)                                            │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │      impressions ████████████████████████████████             │  │
│  │           clicks ████████                                      │  │
│  │      conversions ██                                            │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Queue Lag (seconds)                                                 │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ 60│ ─────────────────── Threshold ────────────────────        │  │
│  │   │                                                           │  │
│  │ 30│        ╱╲                                                 │  │
│  │   │   ╱───╯  ╲───╮                                            │  │
│  │  0│───╯          ╰────────────────────────────────────────    │  │
│  └───────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

### Panels

1. **Events Enqueued** (Stat) - Events added to queue per second
   ```promql
   sum(rate(analytics_events_enqueued_total[5m]))
   ```

2. **Events Written** (Stat) - Successful Postgres writes per second
   ```promql
   sum(rate(analytics_events_written_total[5m]))
   ```

3. **Events Failed** (Stat) - Failed events per second with color thresholds
   ```promql
   sum(rate(analytics_events_failed_total[5m]))
   ```

4. **Success Rate %** (Stat) - Write success percentage
   ```promql
   sum(rate(analytics_events_written_total[5m])) 
   / (sum(rate(analytics_events_written_total[5m])) 
      + sum(rate(analytics_events_failed_total[5m]))) 
   * 100
   ```
   **Thresholds:** Green >99%, Yellow 95-99%, Red <95%

5. **Events by Kind (Enqueued)** (Timeseries) - Impressions, clicks, conversions
   ```promql
   sum by (kind) (rate(analytics_events_enqueued_total[5m]))
   ```

6. **Events by Kind (Written)** (Timeseries) - Successful writes by type
   ```promql
   sum by (kind) (rate(analytics_events_written_total[5m]))
   ```

7. **Failed Events by Kind** (Timeseries) - Failure breakdown
   ```promql
   sum by (kind) (rate(analytics_events_failed_total[5m]))
   > 0
   ```

8. **Rate Limited Events** (Timeseries) - Events rejected by rate limiter
   ```promql
   sum(rate(tracking_rate_limited_total[5m]))
   ```

9. **Blocked Events** (Timeseries) - Events blocked by fraud detector
   ```promql
   sum(rate(tracking_blocked_total[5m]))
   ```

10. **Queue Lag** (Timeseries) - Processing delay with 60s threshold line
    ```promql
    redis_queue_lag_seconds
    ```

11. **Event Kind Summary** (Table) - 1h aggregation showing enqueued/written/failed counts
    ```promql
    # Enqueued (last 1h)
    sum by (kind) (increase(analytics_events_enqueued_total[1h]))
    
    # Written (last 1h)
    sum by (kind) (increase(analytics_events_written_total[1h]))
    
    # Failed (last 1h)
    sum by (kind) (increase(analytics_events_failed_total[1h]))
    ```

12. **HEAD Requests** (Timeseries) - Tracking pixel HEAD requests
    ```promql
    sum(rate(tracking_head_total[5m]))
    ```

### Usage

**Investigate Pipeline Backlog:**
1. Check "Queue Lag" panel - if >60s, queue is backing up
2. Compare "Events Enqueued" vs "Events Written" - gap indicates bottleneck
3. Inspect Postgres merges: `psql "$DATABASE_URL" -c "SELECT pid, query, now()-query_start AS age FROM pg_stat_activity WHERE query LIKE '%analytics_%merge%';"`
4. Scale up workers: `kubectl scale deployment/analytics-worker --replicas=5`

**Debug Failed Events:**
1. Check "Failed Events by Kind" - identify which event type is failing
2. Look at "Event Kind Summary" table - calculate failure rate per kind
3. Get error details from logs: `kubectl logs -n analytics deployment/analytics-worker | grep "write failed"`
4. Common causes: Schema mismatch, invalid timestamps, duplicate keys

---

## Database & Queue

**Dashboard ID:** `db-queue`  
**Panels:** 13  
**File:** `monitoring/grafana/db-queue.json`

### Purpose

Monitor PostgreSQL database performance and background job queue health.

### Key Metrics

#### Database Query Latency

```promql
# p95
histogram_quantile(0.95, 
  sum by (le) (rate(db_query_duration_seconds_bucket[5m]))
)

# p99
histogram_quantile(0.99, 
  sum by (le) (rate(db_query_duration_seconds_bucket[5m]))
)
```

**Thresholds:**
- p95: Green <200ms, Yellow 200-500ms, Red >500ms
- p99: Green <500ms, Yellow 500-1000ms, Red >1000ms

#### Connection Pool Utilization

```promql
db_pool_active_connections / db_pool_max_connections * 100
```

Percentage of pool in use. Above 80% suggests need to scale.

#### Queue Depth

```promql
sum by (queue) (queue_depth)
```

Number of jobs waiting in each queue. Sustained high depth (>1000) indicates processing issues.

#### Queue Backlog Age

```promql
time() - queue_oldest_job_timestamp
```

Age of oldest unprocessed job in seconds. Should be <300s (5 minutes).

#### Queue Failure Rate

```promql
sum(rate(queue_jobs_failed_total[5m])) 
/ 
(sum(rate(queue_jobs_completed_total[5m])) + sum(rate(queue_jobs_failed_total[5m]))) * 100
```

Percentage of failed jobs. Should be <5%.

### Dashboard Screenshot

```
┌─────────────────────────────────────────────────────────────────────┐
│ Database & Queue Health Dashboard            [Last 1h] [↻ 30s]      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  DB p95         DB p99         Queue Depth    Backlog Age           │
│  ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐          │
│  │  85ms   │   │  245ms   │   │  1,234   │   │  45s     │          │
│  │  🟢     │   │  🟢      │   │  ↑ +15%  │   │  🟢      │          │
│  └─────────┘   └──────────┘   └──────────┘   └──────────┘          │
│                                                                       │
│  Connection Pool Utilization (%)                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ 100│                                                           │  │
│  │  80│ ─────────────────── Warning ─────────────────────        │  │
│  │    │                              ╱──╲                        │  │
│  │  50│                    ╱────────╯    ╲                       │  │
│  │    │       ╱───────────╯                ╲                     │  │
│  │  20│──────╯                               ╲─────────────      │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  Queue Performance (Last 1h)                                         │
│  ┌────────────────────┬─────────┬───────────┬─────────┬────────┐   │
│  │ Queue              │ Depth   │ Completed │ Failed  │ Rate   │   │
│  ├────────────────────┼─────────┼───────────┼─────────┼────────┤   │
│  │ payment-processing │ 856     │ 45,890    │ 12      │ 0.03%  │   │
│  │ email-sending      │ 234     │ 89,234    │ 5       │ 0.01%  │   │
│  │ analytics-export   │ 144     │ 12,456    │ 0       │ 0.00%  │   │
│  └────────────────────┴─────────┴───────────┴─────────┴────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### Panels

1. **DB Query p95** (Stat) - 95th percentile query latency
   ```promql
   histogram_quantile(0.95, 
     sum by (le) (rate(db_query_duration_seconds_bucket[5m]))
   )
   ```

2. **DB Query p99** (Stat) - 99th percentile with color thresholds
   ```promql
   histogram_quantile(0.99, 
     sum by (le) (rate(db_query_duration_seconds_bucket[5m]))
   )
   ```
   **Thresholds:** Green <500ms, Yellow 500-1000ms, Red >1000ms

3. **Queue Depth** (Stat) - Total jobs waiting across all queues
   ```promql
   sum(queue_depth)
   ```

4. **Backlog Age** (Stat) - Oldest job age with 300s threshold
   ```promql
   max(time() - queue_oldest_job_timestamp)
   ```

5. **DB Query Latency Percentiles** (Timeseries) - p50/p75/p95/p99
   ```promql
   # p50
   histogram_quantile(0.50, sum by (le) (rate(db_query_duration_seconds_bucket[5m])))
   # p75
   histogram_quantile(0.75, sum by (le) (rate(db_query_duration_seconds_bucket[5m])))
   # p95
   histogram_quantile(0.95, sum by (le) (rate(db_query_duration_seconds_bucket[5m])))
   # p99
   histogram_quantile(0.99, sum by (le) (rate(db_query_duration_seconds_bucket[5m])))
   ```

6. **DB Query Latency by Operation** (Timeseries) - SELECT/INSERT/UPDATE/DELETE
   ```promql
   histogram_quantile(0.95, 
     sum by (operation, le) (rate(db_query_duration_seconds_bucket[5m]))
   )
   ```

7. **Connection Pool Utilization** (Timeseries) - % of max connections with 80% line
   ```promql
   db_pool_active_connections / db_pool_max_connections * 100
   ```

8. **Slow Queries** (Table) - Top 10 queries >1s
   ```promql
   topk(10,
     histogram_quantile(0.95, 
       sum by (query, le) (rate(db_query_duration_seconds_bucket[5m]))
     )
   ) > 1.0
   ```

9. **Queue Depth by Queue** (Timeseries) - payment-processing, email-sending, etc.
   ```promql
   sum by (queue) (queue_depth)
   ```

10. **Queue Processing Rate** (Timeseries) - Jobs completed per second
    ```promql
    sum by (queue) (rate(queue_jobs_completed_total[5m]))
    ```

11. **Queue Backlog Growth Rate** (Timeseries) - Derivative of depth (growing/shrinking)
    ```promql
    deriv(sum by (queue) (queue_depth)[5m:])
    ```

12. **Queue Failure Rate** (Timeseries) - Failed job percentage with 10% threshold
    ```promql
    sum by (queue) (rate(queue_jobs_failed_total[5m])) 
    / (sum by (queue) (rate(queue_jobs_completed_total[5m])) 
       + sum by (queue) (rate(queue_jobs_failed_total[5m]))) 
    * 100
    ```

13. **Queue Summary** (Table) - Per-queue stats (depth, completed, failed)
    ```promql
    # Queue depth
    sum by (queue) (queue_depth)
    
    # Jobs completed (last 1h)
    sum by (queue) (increase(queue_jobs_completed_total[1h]))
    
    # Jobs failed (last 1h)
    sum by (queue) (increase(queue_jobs_failed_total[1h]))
    
    # Failure rate
    sum by (queue) (increase(queue_jobs_failed_total[1h]))
    / (sum by (queue) (increase(queue_jobs_completed_total[1h])) 
       + sum by (queue) (increase(queue_jobs_failed_total[1h])))
    * 100
    ```

### Usage

**Optimize Database Performance:**
1. Check "DB Query p99" - if red, investigate slow queries
2. Review "Slow Queries" table - identify queries >1s
3. Click query to filter "DB Query Latency by Operation" panel
4. Common fixes:
   - Add missing indexes
   - Optimize WHERE clauses
   - Use connection pooling
   - Add read replicas

**Resolve Queue Backlog:**
1. Check "Queue Depth by Queue" - identify which queue is backed up
2. Review "Queue Backlog Growth Rate" - is it growing or shrinking?
3. Check "Queue Failure Rate" - high failures cause retries
4. Actions:
   - Scale workers: `kubectl scale deployment/worker --replicas=10`
   - Increase batch size in worker config
   - Clear failed jobs: `redis-cli DEL failed:queue_name`

---

## Dashboard Customization

### Import Dashboard

**Manual Import:**
```bash
# Copy dashboard JSON
cat monitoring/grafana/rtb-overview.json

# In Grafana UI:
# 1. Click "+" → Import
# 2. Paste JSON
# 3. Select Prometheus data source
# 4. Click "Import"
```

**Kubernetes Provisioning:**
```yaml
# grafana-dashboard-configmap.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: rtb-overview-dashboard
  namespace: monitoring
  labels:
    grafana_dashboard: "1"
data:
  rtb-overview.json: |
    {
      "dashboard": { ... }
    }
```

Apply: `kubectl apply -f grafana-dashboard-configmap.yaml`

Grafana sidecar will auto-load dashboards with label `grafana_dashboard: "1"`.

### Clone and Modify

```bash
# Export existing dashboard (Hetzner Production)
curl -X GET http://46.62.252.96:3001/api/dashboards/uid/apex-rtb-analytics \
  -H "Authorization: Bearer $GRAFANA_API_KEY" > my-dashboard.json

# Edit dashboard
vi my-dashboard.json

# Import modified version
curl -X POST http://46.62.252.96:3001/api/dashboards/db \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -H "Content-Type: application/json" \
  -d @my-dashboard.json
```

### Add Variables

Create variables for filtering dashboards:

```json
{
  "templating": {
    "list": [
      {
        "name": "environment",
        "type": "custom",
        "query": "dev,staging,production",
        "current": {
          "value": "production"
        }
      },
      {
        "name": "adapter",
        "type": "query",
        "query": "label_values(rtb_wins_total, adapter)",
        "datasource": "Prometheus"
      }
    ]
  }
}
```

Use in queries: `rtb_wins_total{environment="$environment", adapter="$adapter"}`

---

## Common Patterns

### Time Range Selector

All dashboards support time range selection via dropdown:

- Last 5 minutes
- Last 15 minutes
- Last 1 hour (default)
- Last 6 hours
- Last 24 hours
- Custom range

### Auto-Refresh

Dashboards auto-refresh based on refresh interval:

- API RED: 10s
- RTB/Tracking/DB: 30s

Change via dropdown in top-right corner.

### Drill-Down

Click on panel legend or table row to filter other panels:

1. Click adapter name in "Wins by Adapter" legend
2. All panels now filter to that adapter
3. Clear filter via "Remove filter" link at top

### Annotations

Add annotations to mark deployments/incidents:

```bash
# Hetzner Production Grafana
curl -X POST http://46.62.252.96:3001/api/annotations \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "dashboardId": 1,
    "time": '$(date +%s)000',
    "text": "Deployed backend v1.2.3",
    "tags": ["deployment", "backend"]
  }'
```

Annotations appear as vertical lines on timeseries panels.

### Thresholds and Colors

Panels use color thresholds to indicate health:

- **Green:** Metric within normal range
- **Yellow:** Warning threshold exceeded (investigate)
- **Red:** Critical threshold exceeded (action required)

Example configuration:
```json
{
  "thresholds": [
    { "value": 0, "color": "green" },
    { "value": 100, "color": "yellow" },
    { "value": 200, "color": "red" }
  ]
}
```

### Prometheus Query Tips

**Rate vs Increase:**
- Use `rate()` for per-second rate: `rate(metric[5m])`
- Use `increase()` for total count: `increase(metric[1h])`

**Aggregation:**
- `sum by (label)` - Group by label
- `topk(10, metric)` - Top 10 highest values
- `bottomk(5, metric)` - Bottom 5 lowest values

**Percentiles:**
```promql
histogram_quantile(0.95, 
  sum by (le, label) (rate(metric_bucket[5m]))
)
```

**Time Shift:**
```promql
# Compare current vs 1 week ago
metric - (metric offset 1w)
```

---

## Related Documentation

- [Alerts Reference](./ALERTS.md)
- [Monitoring Setup](../../monitoring/README.md)
- [Prometheus Configuration](../../monitoring/prometheus.yml)
- [Runbook Index](../runbooks/)
