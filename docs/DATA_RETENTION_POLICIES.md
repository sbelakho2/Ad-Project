# Data Retention Policies

## Overview

This document defines data retention periods for all data types stored in the system. Retention policies balance operational needs, compliance requirements, storage costs, and customer expectations.

## Policy Summary

| Data Type | Retention Period | Automated Cleanup | Rationale |
|-----------|------------------|-------------------|-----------|
| **Analytics Data** | 30 days | ✅ Daily | Performance metrics, sufficient for monthly reports |
| **WAL Archives** | 7 days | ✅ Daily | PITR recovery window, balance safety vs storage |
| **PostgreSQL Logs** | 7 days | ✅ Daily | Debugging window, prevent disk fill |
| **Docker Logs** | 5 rotations × 50MB | ✅ Automatic | Application logs, compressed after rotation |
| **Session Data** | 90 days | ❌ Manual | User sessions, balance UX vs security |
| **Audit Logs** | 1 year | ❌ Manual | Compliance, security investigations |
| **User Accounts** | Indefinite | ❌ Manual | Core data, deleted only on request |
| **Invoices** | 7 years | ❌ Never | Legal/tax requirements (varies by jurisdiction) |
| **Payment Events** | 7 years | ❌ Never | Financial audit trail |
| **Webhook Events** | 30 days | ❌ Manual | Debugging, replay capability |

## Detailed Policies

### 1. Analytics Data (Tiered)

**Raw fact tables (high volume):** `analytics_impressions`, `analytics_revenue_events`, `analytics_clicks`

**Rollup tables (low volume):** `analytics_impression_rollups`, `analytics_click_rollups`, `analytics_revenue_rollups`, `analytics_metrics_rollups`

**Retention:**
- Raw facts: 14 days by default (partition drop)
- Rollups: 5 years by default (monthly partition drop)

**Implementation:** ✅ Automated via partition management scripts (cron)
- Raw partitions are daily RANGE partitions on `observed_at`.
- Rollups are monthly RANGE partitions on `bucket_date`.

**Justification:**
- Raw facts are only needed for short-window debugging and “recent” queries.
- Long-term reporting uses rollups (orders of magnitude smaller).
- Partition drops are O(1) and avoid expensive DELETEs.

**Configuration:**
- Raw partition lifecycle:
  - Create future partitions: `scripts/ops/create_future_partitions.sh`
  - Drop old partitions: `scripts/ops/partition_retention.sh` (default 14 days)
- Rollup lifecycle:
  - Create monthly partitions: `scripts/ops/create_future_rollup_partitions.sh`
  - Drop old rollup partitions: `scripts/ops/rollup_retention.sh` (default 5 years)

**Manual Override (raw facts):**
```bash
# Keep 30 days of raw instead of 14
./scripts/ops/partition_retention.sh 30

# Emergency cleanup - keep only 7 days
./scripts/ops/partition_retention.sh 7
```

### 2. WAL Archives (7 days)

**Location:** `/var/backups/postgresql/wal/` on DB Server

**Retention:** 7 days from archive date

**Implementation:** ✅ Automated via daily cron
```bash
# Runs daily via cron
# /etc/cron.d/apex-wal-cleanup
/usr/local/bin/pg_wal_cleanup.sh
```

**Justification:**
- 7 days provides reasonable PITR recovery window
- Balance between safety and storage costs (note: WAL volume depends heavily on ingest/write patterns)
- Most incidents detected within 48 hours

**Operational Note (Observed 2025-12-31):**
- WAL archives were ~39GB while covering only ~3.5 days of history.
- This indicates WAL generation is high and the 7-day PITR window can become disk-prohibitive.

**Mitigation (preferred before reducing retention):**
- Enable WAL compression (`wal_compression=zstd`) and reduce checkpoint churn (increase `max_wal_size`, increase `checkpoint_timeout`).
- Re-evaluate WAL archive growth rate after 24 hours before changing retention.

**Configuration:**
```bash
# Edit /usr/local/bin/pg_wal_cleanup.sh
# (Or adjust via your configuration management)

# Check current usage
du -sh /var/backups/postgresql/wal/
```

**Extend Retention:**
```bash
# Keep 14 days instead of 7
sed -i 's/-mtime +7/-mtime +14/' /usr/local/bin/pg_wal_cleanup.sh
```

### 3. PostgreSQL Logs (7 days)

**Location:** `/var/log/postgresql/` on DB Server

**Retention:** 7 days, compressed logs kept 14 days

**Implementation:** ✅ Automated via logrotate
```bash
# Configuration: /etc/logrotate.d/postgresql-custom
# Runs daily via system cron
```

**Justification:**
- Debugging window for database issues
- Prevents disk fill (learned from 61GB log incident!)
- Compressed logs provide extended history

**Configuration:**
```bash
# Edit /etc/logrotate.d/postgresql-custom
/var/lib/postgresql/16/main/log/*.log {
    daily
    rotate 7           # Keep 7 days uncompressed
    maxsize 100M       # Force rotation at 100MB
    compress
    delaycompress
    notifempty
}

# Compressed logs (*.gz) deleted after 14 days
```

### 4. Docker Logs (5 rotations × 50MB)

**Location:** `/var/lib/docker/containers/` on App Server

**Retention:** 5 files × 50MB max = 250MB per container

**Implementation:** ✅ Automated via Docker daemon
```json
// /etc/docker/daemon.json
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "50m",
    "max-file": "5",
    "compress": "true"
  }
}
```

**Justification:**
- Application logs for debugging
- Automatic compression after rotation
- Prevents runaway log growth

**Configuration:**
```bash
# Change max-size or max-file
vi /etc/docker/daemon.json
systemctl restart docker
```

### 5. Session Data (90 days) ⚠️ TODO

**Tables:** `sessions`, `refresh_tokens`

**Retention:** 90 days from last activity

**Implementation:** ❌ Not yet automated

**Justification:**
- Balance between user convenience and security
- Most users return within 90 days
- Expired sessions force re-authentication

**Recommended Implementation:**
```sql
-- Create cleanup function
CREATE OR REPLACE FUNCTION cleanup_expired_sessions()
RETURNS void AS $$
BEGIN
  -- Delete sessions older than 90 days
  DELETE FROM sessions 
  WHERE last_activity_at < NOW() - INTERVAL '90 days';
  
  -- Delete expired refresh tokens
  DELETE FROM refresh_tokens 
  WHERE expires_at < NOW();
  
  RAISE NOTICE 'Session cleanup completed';
END;
$$ LANGUAGE plpgsql;

-- Schedule via cron (weekly)
-- 0 3 * * 0 psql -U postgres -d adproject -c "SELECT cleanup_expired_sessions();"
```

**Action Required:**
1. Create cleanup function in database
2. Add weekly cron job to DB server
3. Test cleanup doesn't break active sessions
4. Monitor session table size

### 6. Audit Logs (1 year) ⚠️ TODO

**Tables:** `audit_log`, `transaction_log`, `billing_audit`

**Retention:** 1 year from creation

**Implementation:** ❌ Not yet automated

**Justification:**
- Security investigations
- Compliance requirements (SOC 2, GDPR)
- Financial audit trail

**Recommended Implementation:**
```sql
-- Partition audit logs by month
CREATE TABLE audit_log_2025_12 PARTITION OF audit_log
  FOR VALUES FROM ('2025-12-01') TO ('2026-01-01');

-- Create cleanup function
CREATE OR REPLACE FUNCTION cleanup_old_audit_logs()
RETURNS void AS $$
DECLARE
  cutoff_date date := CURRENT_DATE - INTERVAL '1 year';
BEGIN
  -- Drop partitions older than 1 year
  -- Implementation depends on audit log structure
  RAISE NOTICE 'Audit log cleanup: cutoff date %', cutoff_date;
END;
$$ LANGUAGE plpgsql;
```

**Action Required:**
1. Partition audit_log table by month (similar to analytics_impressions)
2. Create automated cleanup function
3. Add quarterly cron job (manual review before deletion)
4. Document compliance requirements

### 7. User Accounts (Indefinite)

**Tables:** `users`, `publishers`, `apps`, `placements`

**Retention:** Indefinite (deleted only on explicit request)

**Implementation:** Manual deletion only

**Justification:**
- Core business data
- GDPR right to be forgotten requires manual verification
- Cascading deletes must be carefully handled

**Deletion Process:**
1. User requests account deletion
2. Support verifies identity
3. Export user data for records (if required by law)
4. Execute deletion script with transaction safety:

```sql
BEGIN;

-- Archive user data to separate table
INSERT INTO deleted_users_archive
SELECT *, NOW() AS deleted_at FROM users WHERE id = $1;

-- Delete in dependency order
DELETE FROM analytics_impressions WHERE publisher_id = $1;
DELETE FROM analytics_revenue_events WHERE publisher_id = $1;
DELETE FROM subscriptions WHERE customer_id = $1;
DELETE FROM refresh_tokens WHERE user_id = $1;
DELETE FROM sessions WHERE user_id = $1;
DELETE FROM placements WHERE app_id IN (SELECT id FROM apps WHERE publisher_id = $1);
DELETE FROM apps WHERE publisher_id = $1;
DELETE FROM publishers WHERE user_id = $1;
DELETE FROM users WHERE id = $1;

COMMIT;
```

**Compliance:**
- GDPR: 30 days to fulfill deletion request
- CCPA: 45 days to fulfill deletion request
- Retain financial records even after user deletion (legal requirement)

### 8. Invoices & Payment Events (7 years)

**Tables:** `invoices`, `payments`, `stripe_webhook_events`, `transaction_log`

**Retention:** 7 years (legal requirement)

**Implementation:** Never deleted automatically

**Justification:**
- Tax law requirements (varies by jurisdiction):
  - US: 7 years (IRS)
  - EU: 6-10 years (depends on country)
  - UK: 6 years
- Financial audit trail
- Dispute resolution

**Storage Optimization:**
```sql
-- Move invoices older than 2 years to archive partition
CREATE TABLE invoices_archive PARTITION OF invoices
  FOR VALUES FROM ('1970-01-01') TO ('2023-01-01');

-- Compress old partitions
-- Consider pg_dump + external storage for very old data
```

**Action Required:**
1. Document legal retention requirements for your jurisdiction
2. Implement invoice partitioning by year
3. Create archival process for invoices > 2 years old
4. Store archives in cheaper cold storage (S3 Glacier, etc.)

### 9. Webhook Events (30 days) ⚠️ TODO

**Table:** `stripe_webhook_events`

**Retention:** 30 days from receipt

**Implementation:** ❌ Not yet automated

**Justification:**
- Debugging webhook delivery issues
- Replay capability for failed events
- Stripe keeps events for 30 days, we match that

**Recommended Implementation:**
```sql
-- Create cleanup function
CREATE OR REPLACE FUNCTION cleanup_old_webhook_events()
RETURNS void AS $$
BEGIN
  DELETE FROM stripe_webhook_events 
  WHERE created_at < NOW() - INTERVAL '30 days';
  
  RAISE NOTICE 'Webhook events cleanup completed';
END;
$$ LANGUAGE plpgsql;

-- Schedule via cron (weekly)
-- 0 4 * * 0 psql -U postgres -d adproject -c "SELECT cleanup_old_webhook_events();"
```

**Action Required:**
1. Create cleanup function
2. Add weekly cron job
3. Test doesn't break active webhook processing

## Implementation Checklist

### ✅ Completed
- [x] Analytics data: 30 days automated cleanup
- [x] WAL archives: 7 days automated cleanup  
- [x] PostgreSQL logs: 7 days automated rotation
- [x] Docker logs: 250MB per container automatic rotation

### ⚠️ Pending Implementation
- [ ] Session data: 90 days cleanup function + cron
- [ ] Audit logs: 1 year retention with partitioning
- [ ] Webhook events: 30 days cleanup function + cron
- [ ] Invoice archival: Cold storage for > 2 years
- [ ] User deletion: Automated GDPR compliance workflow

### 📝 Documentation Needed
- [ ] Legal retention requirements for your jurisdiction
- [ ] GDPR/CCPA compliance procedures
- [ ] Data export formats for user requests
- [ ] Disaster recovery procedures
- [ ] Data breach response plan

## Monitoring Retention Policy Compliance

### Check Analytics Retention
```sql
-- Should only show partitions within 30 days
SELECT tablename, pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename))
FROM pg_tables 
WHERE tablename LIKE 'analytics_impressions_%' 
ORDER BY tablename DESC;
```

### Check WAL Archive Size
```bash
# Should be ~4-5GB for 7 days
du -sh /var/backups/postgresql/wal/
ls -lah /var/backups/postgresql/wal/ | tail -20
```

### Check PostgreSQL Log Size
```bash
# Should not exceed ~700MB (7 days × 100MB)
du -sh /var/log/postgresql/
du -sh /var/lib/postgresql/16/main/log/
```

### Check Docker Log Size
```bash
# Should not exceed 250MB per container
docker ps -q | xargs -I {} sh -c 'echo {}; du -sh /var/lib/docker/containers/{}'
```

## Emergency Cleanup Procedures

### Analytics Data Filling Disk
```sql
-- Emergency: keep only 7 days
SELECT drop_old_analytics_impressions_partitions(7);

-- Check savings
SELECT pg_size_pretty(pg_database_size('adproject'));
```

### WAL Archives Filling Disk
```bash
# Emergency: delete archives older than 3 days
find /var/backups/postgresql/wal/ -type f -mtime +3 -delete
```

### PostgreSQL Logs Filling Disk
```bash
# Emergency: delete logs older than 3 days
find /var/lib/postgresql/16/main/log/ -name "*.log" -mtime +3 -delete
find /var/log/postgresql/ -name "*.log" -mtime +3 -delete
```

### Full Disk Recovery
See [disk space monitoring script](../backend/scripts/check-disk-space.sh) which automatically:
1. Deletes logs > 3 days if disk > 80%
2. Compresses logs > 1 day
3. Always deletes logs > 7 days
4. Alerts on disk usage

## Compliance Considerations

### GDPR (EU)
- **Right to erasure**: Must delete user data within 30 days of request
- **Right to portability**: Must export user data in machine-readable format
- **Retention limits**: Cannot keep personal data longer than necessary
- **Audit trail**: Keep deletion records for compliance

### CCPA (California)
- **Right to deletion**: Must delete user data within 45 days of verified request
- **Exceptions**: Can retain data for legal/contractual purposes
- **Audit trail**: Document deletion process

### Financial Records
- **Retention required**: 7 years for tax purposes (US)
- **Cannot delete**: Invoices, payments, transaction logs
- **Even after user deletion**: Financial records must be retained

### Security Logs
- **Incident response**: Retain logs for 1 year minimum
- **Compliance**: SOC 2 requires audit log retention
- **Legal hold**: May need to preserve logs during litigation

## Scheduled Maintenance

### Daily (Automated)
- 2:00 AM UTC: Disk space monitoring and cleanup
- 3:00 AM UTC: WAL archive cleanup (7 days)
- 4:00 AM UTC: Analytics partition creation (7 days ahead)
- Daily: PostgreSQL log rotation (7 days)
- Continuous: Docker log rotation (250MB limit)

### Weekly (To be implemented)
- Sunday 3:00 AM UTC: Session cleanup (90 days)
- Sunday 4:00 AM UTC: Webhook events cleanup (30 days)

### Monthly (Manual)
- First Sunday: Review analytics partition sizes
- First Sunday: Check audit log growth rate
- First Sunday: Verify all cron jobs running

### Quarterly (Manual)
- Review legal retention requirements
- Audit log archival planning
- Invoice cold storage migration
- Disaster recovery test

### Annually (Manual)
- Update compliance documentation
- Review retention policies
- Test GDPR deletion workflow
- Financial records archival

## Cost Optimization

### Current Storage Costs (Estimated)
- **Analytics partitions**: ~330MB for 30 days
- **WAL archives**: ~4.3GB for 7 days
- **PostgreSQL logs**: ~700MB for 7 days
- **Docker logs**: ~1GB across all containers
- **Total active**: ~6.5GB operational data

### Potential Savings
1. **Reduce analytics to 14 days**: Save ~165MB (~50%)
2. **Reduce WAL to 3 days**: Save ~2.5GB (~60%)
3. **Archive old invoices to S3**: Save database space
4. **Compress old audit logs**: 70-90% compression ratio

### Storage Growth Projections
With 100k impressions/day:
- Analytics: ~1GB/month
- WAL: Stable at ~4GB
- Logs: Stable at ~2GB
- Total: ~7GB operational, linear growth in analytics

Plan to review retention policies when database exceeds 50GB.
