# ApexMediation Monitoring Infrastructure

## Overview

Complete GDPR-compliant monitoring stack deployed on Hetzner infrastructure with deep analytics, alerting, and visualization.

## Access URLs

| Service | URL | Credentials |
|---------|-----|-------------|
| **Prometheus** | http://46.62.252.96:9090 | No auth required (internal) |
| **Grafana** | http://46.62.252.96:3001 | `admin` / `ApexGrafana2025!` |
| **Alertmanager** | http://46.62.252.96:9093 | No auth required (internal) |
| **Node Exporter** | http://46.62.252.96:9100 | N/A (metrics only) |
| **Postgres Exporter** | http://46.62.252.96:9187 | N/A (metrics only) |
| **cAdvisor** | http://46.62.252.96:8081 | N/A (metrics only) |

## Scrape Targets (All Healthy ✅)

| Target | Endpoint | Scrape Interval | Status |
|--------|----------|-----------------|--------|
| `backend-api` | adproject-backend-1:8080/metrics | 15s | ✅ UP |
| `billing` | adproject-backend-1:8080/metrics | 15s | ✅ UP |
| `rtb-auction` | adproject-backend-1:8080/metrics | 15s | ✅ UP |
| `postgres` | postgres-exporter:9187/metrics | 15s | ✅ UP |
| `node-app-server` | node-exporter:9100/metrics | 15s | ✅ UP |
| `cadvisor` | cadvisor:8080/metrics | 15s | ✅ UP |
| `prometheus` | localhost:9090/metrics | 15s | ✅ UP |

## Grafana Dashboards

### 1. Executive Overview (`apex-executive-overview`)
- Platform health status (API uptime, error rate, P95 latency)
- Request rate time series
- Error rate gauge
- Database connection monitoring
- Key business metrics overview

### 2. RTB Analytics (`apex-rtb-analytics`)
- Fill rate by adapter
- Auction P95 latency trends
- Revenue by adapter (hourly/daily)
- Bid price distribution histograms
- Targeting breakdown (format, platform, geo)
- Adapter response time analysis

### 3. Billing & Finance (`apex-billing-finance`)
- MRR and ARR tracking
- MRR growth rate (30-day)
- Monthly churn rate
- Payment success/failure rates
- Payment processing latency
- Webhook delivery status
- Revenue breakdown by plan

### 4. Database Performance (`apex-database`)
- PostgreSQL connection pool status
- Database size monitoring (21MB adproject)
- Table size breakdown
- Index usage statistics
- Transaction rate (commits/rollbacks)
- Lock monitoring
- I/O and checkpoint analysis
- Replication lag (if applicable)

## Alert Rules

### Backend Alerts (`alerts/backend.yml`)
- `HighErrorRate` - API error rate > 5% for 5 minutes
- `HighLatency` - P95 latency > 500ms for 5 minutes
- `ApiDown` - Backend unreachable for 2 minutes
- `HighMemoryUsage` - Node.js memory > 1.5GB

### Database Alerts (`alerts/database.yml`)
- `PostgresDown` - Database unreachable
- `CriticalDatabaseConnections` - Pool > 90% utilized
- `HighReplicationLag` - Lag > 30 seconds
- `TableBloat` - Dead rows > 10% of live rows
- `LongRunningQueries` - Queries > 5 minutes

### Billing Alerts (`alerts/billing.yml`)
- `HighPaymentFailureRate` - Failures > 10%
- `WebhookDeliveryFailure` - Webhook failures > 5%
- `MRRDropAlert` - MRR decrease > 10%
- `PaymentProcessingDelay` - P95 > 10 seconds

### Infrastructure Alerts (`alerts/infrastructure.yml`)
- `DiskSpaceLow` - Disk > 85% used
- `HighMemoryUsage` - Memory > 90%
- `HighCPU` - CPU > 80% sustained
- `SSLCertificateExpiry` - Cert expires in < 30 days

## GDPR Compliance

### PII Protection Measures

1. **No User Identifiers in Metrics**
   - User IDs, emails, and IP addresses are explicitly dropped via `metric_relabel_configs`
   - Only anonymized aggregate counts are collected

2. **Query Text Not Collected**
   - PostgreSQL query texts are not exposed to prevent PII leakage
   - Only query performance statistics (duration, count) are tracked

3. **Alert Annotations**
   - No user data in alert messages
   - Only system-level metrics referenced

4. **Log Aggregation**
   - Tenant isolation enabled
   - Automatic PII redaction configured

5. **Retention Policies**
   - 30-day metric retention
   - 10GB storage limit with automatic cleanup

## Recording Rules

Pre-computed metrics for efficient dashboard queries:

```yaml
# RTB Metrics
rtb:fill_rate:5m          # Aggregated fill rate
rtb:ecpm:5m               # Average eCPM calculation
rtb:adapter_response_time_p95:5m

# API Metrics  
api:success_rate:5m       # 1 - error_rate
api:latency_p99:5m        # 99th percentile latency
api:request_rate:5m       # Requests per second

# Database Metrics
pg:connection_utilization # Pool usage percentage
pg:cache_hit_ratio:5m     # Buffer cache effectiveness
pg:transaction_rate:5m    # Commits per second
```

## Backend Integration

The backend service has been configured with:

```yaml
environment:
  - PROMETHEUS_URL=http://prometheus:9090
  - GRAFANA_URL=http://grafana:3000
networks:
  - apex-net
  - monitoring_monitoring  # Connected to monitoring stack
```

### Admin Health Endpoint

The `/api/v1/admin/health/red` endpoint now returns real Prometheus metrics:

```json
{
  "success": true,
  "data": {
    "api_rps_5m": 0.357,
    "api_error_rate_5m": 0,
    "api_p95_latency_5m": 0.024,
    "rtb_p95_latency_5m": null,
    "rtb_adapter_timeouts_rps_5m": 0,
    "timestamp": "2025-12-13T09:42:36.760Z"
  }
}
```

## File Structure

```
infrastructure/production/monitoring/
├── docker-compose.monitoring.yml    # Main stack definition
├── prometheus.yml                    # Prometheus configuration
├── alertmanager.yml                  # Alertmanager routing
├── recording-rules.yml               # Pre-computed metrics
├── postgres-exporter-queries.yml     # Custom DB queries
├── alerts/
│   ├── backend.yml                   # API alerts
│   ├── database.yml                  # PostgreSQL alerts
│   ├── billing.yml                   # Payment alerts
│   └── infrastructure.yml            # System alerts
└── grafana/
    ├── provisioning/
    │   ├── datasources/
    │   │   └── datasources.yml       # Prometheus datasource
    │   └── dashboards/
    │       └── dashboards.yml        # Dashboard provisioning
    └── dashboards/
        ├── executive-overview.json
        ├── rtb-analytics.json
        ├── billing-finance.json
        └── database-performance.json
```

## Quick Commands

### Check Status
```bash
# View all monitoring containers
docker ps --format 'table {{.Names}}\t{{.Status}}' | grep -E 'prometheus|grafana|alertmanager|exporter|cadvisor'

# Check Prometheus targets
curl -s http://46.62.252.96:9090/api/v1/targets | jq '.data.activeTargets[].health'

# Query a metric
curl -s "http://46.62.252.96:9090/api/v1/query?query=pg_database_size_size_bytes" | jq
```

### Restart Services
```bash
cd /root/monitoring
docker compose -f docker-compose.monitoring.yml restart prometheus grafana alertmanager
```

### View Logs
```bash
docker logs prometheus --tail 50
docker logs grafana --tail 50
docker logs postgres-exporter --tail 50
```

## Security Notes

- Grafana admin password: `ApexGrafana2025!` (change in production)
- Prometheus and Alertmanager have no auth (internal network only)
- Consider adding nginx reverse proxy with SSL for external access
- Alert notifications should be configured with actual SMTP/Slack credentials

---

**Last Updated:** 2025-12-13
**Deployed By:** Automated monitoring stack configuration
**Version:** 1.0.0
