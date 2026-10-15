# Pre-Deployment Checklist - ApexMediation
**Date:** 2025-11-04  
**Target Environment:** Production  
**Sign-off Required:** Platform Engineering + Finance + Security

---

## 1. Infrastructure Verification

### Cloud Resources
- [ ] **Kubernetes cluster:** EKS/GKE 1.28+ provisioned and accessible
- [ ] **PostgreSQL:** RDS/Cloud SQL 15+ with multi-AZ enabled
- [ ] **ClickHouse:** 23+ deployed with replication configured
- [ ] **Redis:** ElastiCache/Memorystore 7+ with persistence enabled
- [ ] **Load Balancer:** ALB/NLB configured with health checks
- [ ] **CDN:** CloudFront/Cloud CDN pointing to API/Console origins
- [ ] **DNS:** Route53/Cloud DNS records created and propagated
- [ ] **SSL Certificates:** Let's Encrypt or ACM certs issued and valid

### Network & Security
- [ ] **VPC:** Private subnets for databases, public for load balancers
- [ ] **Security Groups:** Inbound rules limited to required ports only
- [ ] **NAT Gateway:** Enabled for private subnet internet access
- [ ] **Bastion Host:** Configured for SSH access (if needed)
- [ ] **WAF:** Rules configured for rate limiting and OWASP Top 10
- [ ] **DDoS Protection:** Shield Standard/Advanced enabled

### Monitoring & Logging
- [ ] **Prometheus:** Deployed and scraping all services
- [ ] **Grafana:** Dashboards imported and accessible
- [ ] **Loki:** Log aggregation configured
- [ ] **Alertmanager:** Routes configured for Slack/PagerDuty/Email
- [ ] **Backup Jobs:** Automated PostgreSQL/ClickHouse backups scheduled

**Sign-off:** _________________ (Infrastructure Lead)

---

## 2. Database Preparation

### PostgreSQL
- [ ] **Migrations:** All 14 migrations applied successfully
  ```bash
  psql $DATABASE_URL -c "SELECT * FROM schema_migrations ORDER BY version;"
  # Expected: 001 through 014 all present
  ```
- [ ] **Indexes:** All indexes created (check `pg_indexes`)
- [ ] **Users:** Application user with least-privilege grants
- [ ] **Read Replica:** Configured for analytics queries
- [ ] **Connection Pooling:** PgBouncer or RDS Proxy enabled
- [ ] **Backup Retention:** Point-in-time recovery enabled; retention meets the environment requirement (e.g. >=24h on Hetzner production)
- [ ] **Performance Tuning:**
  ```sql
  -- Verify settings
  SHOW shared_buffers;        -- Should be ~25% of RAM
  SHOW effective_cache_size;  -- Should be ~75% of RAM
  SHOW work_mem;             -- Should be 4MB-16MB
  SHOW maintenance_work_mem; -- Should be 256MB+
  ```

### ClickHouse
- [ ] **Schema:** All tables created with proper TTL
- [ ] **Partitions:** Monthly partitioning configured
- [ ] **Materialized Views:** All 4 views created and populating
- [ ] **Replication:** Configured if using distributed tables
- [ ] **Retention:** Old partitions dropping automatically
- [ ] **Performance:**
  ```sql
  -- Verify settings
  SELECT * FROM system.settings WHERE name IN ('max_memory_usage', 'max_threads');
  ```

### Redis
- [ ] **Persistence:** RDB + AOF enabled
- [ ] **Eviction Policy:** `allkeys-lru` configured
- [ ] **Max Memory:** Set to 80% of instance RAM
- [ ] **Connection Limit:** Increased for production load
- [ ] **Sentinel/Cluster:** High availability configured

**Sign-off:** _________________ (Database Admin)

---

## 3. Application Deployment

### Backend API
- [ ] **Docker Image:** Built and pushed to registry
  ```bash
  docker images | grep apexmediation-backend
  # Expected: Tag matching git SHA or release version
  ```
- [ ] **Environment Variables:** All 25+ variables set correctly
  ```bash
  kubectl get secret backend-secrets -n production -o yaml
  # Verify: DATABASE_URL, OPENAI_API_KEY, APEX_ADYEN_API_KEY, etc.
  ```
- [ ] **Resource Limits:** CPU/Memory requests and limits defined
- [ ] **Health Checks:** `/health` endpoint returns 200
- [ ] **Readiness Probe:** Service ready before receiving traffic
- [ ] **Liveness Probe:** Auto-restart on unhealthy state
- [ ] **Replicas:** Minimum 3 pods for high availability
- [ ] **HPA:** Horizontal Pod Autoscaler configured (3-10 replicas)
- [ ] **PDB:** PodDisruptionBudget set (minAvailable: 2)

### Console (Next.js)
- [ ] **Docker Image:** Built and pushed to registry
- [ ] **Static Assets:** Bundled and optimized
- [ ] **Environment Variables:** `NEXT_PUBLIC_API_URL` set correctly
- [ ] **Authentication:** NextAuth configured with production secrets
- [ ] **Replicas:** Minimum 2 pods
- [ ] **CDN:** Static assets served via CloudFront/CDN

### Background Jobs
- [ ] **Cron Jobs:** Scheduled in Kubernetes CronJobs
  ```bash
  kubectl get cronjobs -n production
  # Expected: sales-automation, growth-engine, self-evolving-system
  ```
- [ ] **Job Schedules:** Verified correct (sales: daily 7pm, growth: daily 7pm, etc.)
- [ ] **Job History:** SuccessfulJobsHistoryLimit: 3, FailedJobsHistoryLimit: 1
- [ ] **Timeouts:** activeDeadlineSeconds set to prevent runaway jobs

**Sign-off:** _________________ (Platform Engineering Lead)

---

## 4. AI Automation Cost Controls

### OpenAI Configuration
- [ ] **API Key:** Production key stored in secrets manager
- [ ] **Usage Limits:** Configured in OpenAI dashboard
  - Soft limit (50%): Email to platform + finance
  - Hard limit (100%): API blocked + Slack alert
- [ ] **Monthly Budget:** Approved by finance ($100/month initial)
- [ ] **Feature Flags:** All set to `false` initially
  ```bash
  kubectl get secret backend-secrets -n production -o jsonpath='{.data}' | grep ENABLE
  # Expected: ENABLE_AI_AUTOMATION=false (base64 encoded)
  ```

### Staged Rollout Plan
- [ ] **Week 1:** Enable `ENABLE_SALES_AI_OPTIMIZATION` only
- [ ] **Week 2:** Add `ENABLE_GROWTH_AI_ANALYTICS` if spend < $50
- [ ] **Week 3:** Add `ENABLE_SELF_EVOLVING_AI` if spend < $75
- [ ] **Rollback Procedure:** Documented in runbook

### Monitoring
- [ ] **Prometheus Alerts:** AI cost alerts added to `monitoring/alerts.yml`
- [ ] **Grafana Dashboard:** AI cost tracking panel created
- [ ] **Daily Reviews:** Calendar reminder set for platform team
- [ ] **Monthly Report:** Template prepared for finance review

**Sign-off:** _________________ (Finance Approver)

---

## 5. Security Hardening

### Secrets Management
- [ ] **Kubernetes Secrets:** All sensitive data in secrets (not ConfigMaps)
- [ ] **Secret Rotation:** Process documented for JWT, API keys, DB passwords
- [ ] **RBAC:** ServiceAccounts with minimal permissions
- [ ] **Network Policies:** Pod-to-pod communication restricted
- [ ] **Image Scanning:** Container images scanned for vulnerabilities
  ```bash
  trivy image apexmediation-backend:latest
  # Expected: No HIGH or CRITICAL vulnerabilities
  ```

### API Security
- [ ] **Rate Limiting:** Express rate limiter configured (100 req/15min)
- [ ] **CORS:** Restricted to console domain only
- [ ] **Helmet:** Security headers enabled
- [ ] **Input Validation:** Zod schemas on all endpoints
- [ ] **SQL Injection:** Parameterized queries only (no string concatenation)
- [ ] **JWT Expiration:** Set to 7 days with refresh tokens

### Compliance
- [ ] **GDPR:** Data export endpoints implemented
- [ ] **COPPA:** Age verification enabled
- [ ] **Audit Logs:** All sensitive actions logged to `audit_logs` table
- [ ] **Data Retention:** TTL policies match legal requirements
- [ ] **Privacy Policy:** Updated and linked in console

**Sign-off:** _________________ (Security Officer)

---

## 6. Performance Optimization

### Database Tuning
- [ ] **Connection Pool:** Size = (CPU cores × 2) + effective_spindle_count
- [ ] **Query Performance:** All slow queries (>1s) indexed
- [ ] **Vacuum:** Autovacuum enabled with aggressive settings
- [ ] **Statistics:** `ANALYZE` run on all tables

### Caching Strategy
- [ ] **Redis Cache:** TTLs set appropriately
  - SDK config: 24h
  - Session data: 30min
  - Dashboard metrics: 5min
- [ ] **CDN Cache:** Static assets cached for 1 year
- [ ] **HTTP Caching:** Cache-Control headers on API responses

### Code Optimization
- [ ] **Database Queries:** N+1 queries eliminated
- [ ] **Async Operations:** Long tasks moved to background jobs
- [ ] **Pagination:** Implemented on all list endpoints
- [ ] **Compression:** Gzip enabled for API responses

**Sign-off:** _________________ (Performance Engineer)

---

## 7. Observability & Alerting

### Metrics Collection
- [ ] **Prometheus Exporters:** Enabled on all services
- [ ] **Custom Metrics:** Business metrics instrumented
  - `customers_created_total`
  - `revenue_cents`
  - `sdk_crashes_total`
  - `openai_monthly_spend_dollars`
- [ ] **Service Mesh:** Istio metrics flowing to Prometheus

### Alerting Rules
- [ ] **Critical Alerts:** Configured and tested
  - Backend API Down (2min threshold)
  - Database Connection Pool Exhausted
  - High Error Rate (>5%)
  - OpenAI Hard Limit Reached
- [ ] **Warning Alerts:** Configured
  - High Memory Usage (>80%)
  - Slow Queries (>1s)
  - High Churn Rate (>20%)
- [ ] **Alert Routing:** Slack #platform-alerts + PagerDuty

### Dashboards
- [ ] **System Health:** CPU, Memory, Disk, Network
- [ ] **Application Performance:** Latency, Error Rate, Throughput
- [ ] **Business Metrics:** Revenue, Customers, Churn
- [ ] **AI Cost Tracking:** OpenAI spend by service

**Sign-off:** _________________ (SRE Lead)

---

## 8. Disaster Recovery

### Backup Verification
- [ ] **PostgreSQL Backups:** Automated daily backups enabled
  ```bash
  aws rds describe-db-snapshots --db-instance-identifier apexmediation-prod
  # Expected: Recent snapshots present
  ```
- [ ] **ClickHouse Backups:** Daily exports to S3/GCS
- [ ] **Redis Backups:** RDB snapshots enabled (daily)
- [ ] **Configuration Backups:** All Kubernetes manifests in git
- [ ] **Secret Backups:** Secrets Manager versioning enabled

### Recovery Testing
- [ ] **Database Restore:** Tested on staging environment
- [ ] **Point-in-Time Recovery:** Tested (restore to 1 hour ago)
- [ ] **Disaster Recovery Runbook:** Documented and reviewed
- [ ] **RTO/RPO Defined:** Recovery Time Objective = 1 hour, Recovery Point Objective = 15 minutes

### Incident Response
- [ ] **On-Call Schedule:** Configured in PagerDuty
- [ ] **Escalation Policy:** Primary → Secondary → Manager
- [ ] **Incident Templates:** Pre-written for common scenarios
- [ ] **Post-Mortem Process:** Documented and communicated

**Sign-off:** _________________ (Operations Manager)

---

## 9. Business Continuity

### Documentation
- [ ] **API Documentation:** OpenAPI spec up-to-date
- [ ] **Architecture Diagrams:** System design documented
- [ ] **Runbooks:** Created for all operational tasks
  - Deployment procedure
  - Rollback procedure
  - AI cost control
  - Database maintenance
  - Incident response
- [ ] **Onboarding Guide:** For new team members

### Communication Plan
- [ ] **Status Page:** Configured (e.g., status.apexmediation.ee)
- [ ] **Incident Communication:** Templates for customer notifications
- [ ] **Stakeholder Updates:** Schedule for weekly reports

### Legal & Compliance
- [ ] **Terms of Service:** Reviewed by legal
- [ ] **Privacy Policy:** GDPR/CCPA compliant
- [ ] **SLA Commitments:** Documented (99.9% uptime)
- [ ] **Data Processing Agreement:** Signed with customers

**Sign-off:** _________________ (Business Operations)

---

## 10. Go-Live Checklist

### Pre-Launch (T-24 hours)
- [ ] **Smoke Tests:** All critical user flows tested on staging
- [ ] **Load Testing:** Simulated 10x expected traffic successfully
- [ ] **DNS TTL:** Reduced to 60 seconds for quick rollback
- [ ] **Team Availability:** All engineers on standby
- [ ] **Customer Communication:** Announcement drafted

### Launch (T-0)
- [ ] **DNS Cutover:** Updated A/CNAME records
- [ ] **SSL Verification:** HTTPS working on all domains
- [ ] **Health Checks:** All services reporting healthy
- [ ] **First Transaction:** Test customer signup completed
- [ ] **Monitoring:** All dashboards green

### Post-Launch (T+1 hour)
- [ ] **Error Rates:** Stable (<1%)
- [ ] **Latency:** P95 < 500ms
- [ ] **Database Performance:** No slow queries
- [ ] **No Critical Alerts:** All systems nominal

### Post-Launch (T+24 hours)
- [ ] **Customer Signups:** At least 1 organic signup
- [ ] **Revenue Events:** First ad impressions tracked
- [ ] **AI Automation:** Logs show successful runs
- [ ] **No Incidents:** Zero production incidents
- [ ] **Retrospective:** Post-launch review scheduled

**Sign-off:** _________________ (CTO / Engineering Director)

---

## Approval Signatures

| Role | Name | Signature | Date |
|------|------|-----------|------|
| Platform Engineering Lead | _____________ | _____________ | ____/____/____ |
| Database Admin | _____________ | _____________ | ____/____/____ |
| Security Officer | _____________ | _____________ | ____/____/____ |
| Finance Approver | _____________ | _____________ | ____/____/____ |
| Operations Manager | _____________ | _____________ | ____/____/____ |
| CTO / Engineering Director | _____________ | _____________ | ____/____/____ |

---

## Post-Deployment Monitoring Schedule

**First 24 Hours:**
- [ ] Hour 1: Active monitoring, all hands on deck
- [ ] Hour 6: First health check review
- [ ] Hour 12: Mid-day status report
- [ ] Hour 24: End-of-day retrospective

**First Week:**
- [ ] Daily stand-ups to review metrics
- [ ] AI automation spend tracking (daily)
- [ ] Customer feedback collection
- [ ] Performance optimization based on real traffic

**First Month:**
- [ ] Weekly reports to stakeholders
- [ ] Capacity planning based on actual growth
- [ ] Cost optimization review
- [ ] Feature flag rollout (AI automation)

---

**Checklist Version:** 1.0  
**Last Updated:** 2025-11-04  
**Next Review:** Before next major deployment
