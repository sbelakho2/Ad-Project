# Sales Automation - Operations Guide (V1)

**ApexMediation Self-Service Sales Automation**  
**Last Updated:** 2025-11-03  
**Status:** Production Ready  
**Type:** Inbound Trial Nurture

> **📢 See Also:** [Autonomous Sales V2](AUTONOMOUS_SALES_V2.md) - Zero-cost outbound prospecting system with Thompson Sampling, Spintax, and Naive Bayes classification

> **⚠️ Architecture Note (January 2026):** For V1 cron jobs (trial reminders, dunning, usage sync), these still run on the **App Server backend** (PM2).
> For V2 autonomous sales (outbound prospecting, campaign execution), **ALL work runs on the Test Server** (203.0.113.11:4001).
> The App Server control-plane is strictly a proxy client for V2.

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Usage Metering](#usage-metering)
4. [Dunning Management](#dunning-management)
5. [Email Automation](#email-automation)
6. [Cron Jobs](#cron-jobs)
7. [API Endpoints](#api-endpoints)
8. [Troubleshooting](#troubleshooting)

---

## Overview

The ApexMediation sales automation system provides complete self-service customer lifecycle management:

- **Usage Metering**: Track SDK usage (impressions, API calls, data transfer)
- **Billing**: Automated Adyen integration with overage charges
- **Dunning**: Smart payment retry logic (3 attempts over 7 days)
- **Email Automation**: Transactional emails via self-hosted Postfix (unlimited)
- **Suspension**: Automatic service suspension after failed payments
- **Reactivation**: Instant reactivation upon successful payment

**Design Goal**: Zero manual intervention for <100 customers, <2 hours/week for 100-500 customers.

---

## Architecture

```
┌─────────────┐
│   SDK       │ ── telemetry ──> POST /api/usage/track
│ (iOS/Android)│                        │
└─────────────┘                        ▼
                            ┌──────────────────────┐
                            │ UsageMeteringService │
                            └──────────┬───────────┘
                                       │
                          ┌────────────┼────────────┐
                          ▼            ▼            ▼
                    ┌─────────┐  ┌──────────┐  ┌─────────┐
                    │PostgreSQL│  │ClickHouse│  │  Adyen │
                    │(billing) │  │(analytics)│  │(metering)│
                    └─────────┘  └──────────┘  └─────────┘

┌──────────────┐
│ Adyen       │ ── webhook ──> invoice.payment_failed
│              │                        │
└──────────────┘                        ▼
                            ┌──────────────────────┐
                            │DunningManagementSvc  │
                            └──────────┬───────────┘
                                       │
                          ┌────────────┼────────────┐
                          ▼            ▼            ▼
                    Retry 1 (1d)  Retry 2 (3d)  Retry 3 (7d)
                          │            │            │
                          └────────────┴────────────┘
                                       │
                                       ▼
                              ┌────────────────┐
                              │  Suspend       │
                              │  (max retries) │
                              └────────────────┘

┌──────────────┐
│ Cron Jobs    │ ── every minute ──> Email Queue Processing
│              │ ── hourly ────────> Usage Limit Checks
│              │ ── daily 2 AM ────> Adyen Usage Sync
│              │ ── daily 3 AM ────> Dunning Retries
│              │ ── daily 9 AM ────> Trial Reminders
└──────────────┘
```

---

## Usage Metering

### How It Works

1. **SDK sends telemetry**: `POST /api/usage/track`
2. **Service records usage**: Inserts into PostgreSQL + ClickHouse
3. **Daily sync to Adyen**: Cron job at 2 AM UTC
4. **Hourly limit checks**: Alert at 80%, 90%, 100%, 110%
5. **Monthly billing**: Adyen automatically charges overages

### Usage Tracking

**Metrics Tracked:**
- `impressions`: Ad impressions served
- `api_calls`: API requests made
- `data_transfer`: Data transferred (GB)

**Example SDK Integration:**

```typescript
// iOS SDK telemetry
func trackImpressions(count: Int) {
    let url = URL(string: "https://api.apexmediation.ee/usage/track")!
    var request = URLRequest(url: url)
    request.httpMethod = "POST"
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    
    let body: [String: Any] = [
        "api_key": apiKey,
        "metric_type": "impressions",
        "quantity": count,
        "metadata": [
            "platform": "iOS",
            "sdk_version": "1.0.0",
            "app_id": bundleId
        ]
    ]
    
    request.httpBody = try? JSONSerialization.data(withJSONObject: body)
    
    URLSession.shared.dataTask(with: request).resume()
}
```

### Platform Fee Reference

**Tier bands:**
- **Starter**: $0 – $10k mediated revenue → **0%** platform fee (free for launch).
- **Growth**: $10,001 – $100k → **2.5%** platform fee.
- **Scale**: $100,001 – $500k → **2.0%** platform fee.
- **Enterprise**: $500k+ → **1.0–1.5%** platform fee (custom minimums allowed).

**Usage example:**
```
Customer: Growth tier (default 2.5%)
Mediated revenue: $50,000 this month
Platform fee: $50,000 × 2.5% = $1,250
Invoice shows: $1,250 platform fee + any add-ons (e.g., white-label console)
```

### Manual Operations

**Check customer usage:**
```bash
psql $DATABASE_URL -c "
SELECT 
  customer_id,
  SUM(CASE WHEN metric_type = 'impressions' THEN quantity ELSE 0 END) as impressions,
  SUM(CASE WHEN metric_type = 'api_calls' THEN quantity ELSE 0 END) as api_calls
FROM usage_records
WHERE recorded_at >= NOW() - INTERVAL '30 days'
GROUP BY customer_id
ORDER BY impressions DESC
LIMIT 10;
"
```

**Force Adyen sync for specific customer:**
```typescript
// In Node.js console
import { usageMeteringService } from './services/billing/UsageMeteringService';
await usageMeteringService.syncUsageToAdyen();
```

---

## Dunning Management

### Payment Retry Logic

**Default Configuration:**
- **Grace Period**: 3 days before first retry
- **Retry Schedule**: Days 1, 3, 7 (after grace period)
- **Max Retries**: 3 attempts
- **Action on Failure**: Suspend subscription and API access

**Timeline Example:**
```
Day 0:  Payment fails
        → Send "Payment Failed" email
        → Grace period begins

Day 3:  Retry Attempt 1
        → If fails: Schedule retry in 1 day

Day 4:  Retry Attempt 2
        → If fails: Schedule retry in 3 days

Day 7:  Retry Attempt 3 (FINAL)
        → If fails: Suspend subscription
        → Send "Service Suspended" email
        → Revoke API access
```

### Email Sequence

**1. Payment Failed (Immediate)**
- Subject: "⚠️ Payment failed - action required"
- Content: Payment amount, update link, retry schedule
- From: noreply@apexmediation.ee

**2. Payment Retry (Before Each Attempt)**
- Subject: "Payment retry X of 3"
- Content: Days until retry, attempts remaining
- Special: Final retry warning for attempt 3

**3. Payment Succeeded (After Successful Retry)**
- Subject: "✅ Payment successful - service restored"
- Content: Confirmation, service status

**4. Subscription Suspended (After Max Retries)**
- Subject: "🚫 Service suspended - payment required"
- Content: Reactivation instructions, support contact

### Manual Operations

**Check dunning status for customer:**
```sql
SELECT 
  id, customer_id, invoice_id, attempt_number, next_retry_at, status
FROM dunning_attempts
WHERE customer_id = 'cus_xxx'
ORDER BY created_at DESC;
```

**Manually retry payment:**
```typescript
import { dunningManagementService } from './services/billing/DunningManagementService';

// Process all pending retries (normally run by cron)
await dunningManagementService.processDunningRetries();
```

**Manually reactivate suspended subscription:**
```typescript
await dunningManagementService.reactivateSubscription(
  'cus_xxx', // customer_id
  'sub_xxx'  // subscription_id
);
```

---

## Email Automation

### Email Provider

**Self-Hosted Postfix** (mail.apexmediation.ee)
- Zero cost - unlimited emails
- DKIM signed with OpenDKIM
- Direct delivery to recipient MX
- Full control over deliverability

> **Note:** Example addresses below use RFC 5737 documentation addresses; replace them with your server addresses before applying them.

**Configuration:**
- MX: mail.apexmediation.ee (203.0.113.11)
- SPF: `v=spf1 ip4:203.0.113.11 ip4:203.0.113.10 -all`
- DKIM: apexmail._domainkey selector
- DMARC: `p=reject`

### Email Templates

**Transactional Emails:**
1. Welcome email (immediate after signup)
2. Trial ending reminders (7d, 3d, 1d before)
3. Payment failed notifications
4. Payment retry reminders
5. Payment succeeded confirmation
6. Subscription suspended notice
7. Usage alerts (80%, 90%, 100%, overage)
8. Monthly usage summary
9. SDK update notifications

**Email Sending Logic:**
```typescript
// Emit event → Queue → Process → Send via Resend
await db.query(`
  INSERT INTO events (event_type, data, created_at)
  VALUES ($1, $2, NOW())
`, ['email.welcome', JSON.stringify(emailData)]);

// Cron job processes queue every minute
// → EmailAutomationService.processEmailQueue()
// → EmailAutomationService.sendWelcomeEmail()
// → resend.emails.send()
```

### Preventing Duplicate Emails

**Email Log Table:**
```sql
-- Prevents sending same email type within 24 hours
SELECT id FROM email_log
WHERE customer_id = 'cus_xxx'
  AND email_type = 'trial_ending_7d'
  AND sent_at >= NOW() - INTERVAL '24 hours';
```

### Manual Operations

**Send test email:**
```typescript
import { emailAutomationService } from './services/email/EmailAutomationService';

// Emit email event
await db.query(`
  INSERT INTO events (event_type, data, created_at)
  VALUES ('email.welcome', $1, NOW())
`, [JSON.stringify({
  to: 'test@example.com',
  customer_id: 'cus_test',
  api_key: 'rsk_test123',
  plan_type: 'starter'
})]);

// Process queue
await emailAutomationService.processEmailQueue();
```

**Check email delivery status:**
```sql
-- Recent emails sent
SELECT customer_id, email_type, sent_at
FROM email_log
ORDER BY sent_at DESC
LIMIT 20;

-- Emails in queue (pending)
SELECT event_type, data->'to' as recipient, created_at
FROM events
WHERE event_type LIKE 'email.%'
  AND processed_at IS NULL
ORDER BY created_at ASC;
```

**Retry failed emails:**
```sql
-- Find failed email events
SELECT id, event_type, error_message
FROM events
WHERE event_type LIKE 'email.%'
  AND processed_at IS NOT NULL
  AND error_message IS NOT NULL
ORDER BY created_at DESC;

-- Reset to retry
UPDATE events
SET processed_at = NULL, error_message = NULL
WHERE id = 12345;
```

---

## Cron Jobs

### Schedule

| Job | Frequency | Time (UTC) | Purpose |
|-----|-----------|-----------|---------|
| Email Queue | Every minute | * * * * * | Process pending email events |
| Usage Limits | Every hour | 0 * * * * | Check usage and send alerts |
| Adyen Sync | Daily | 2:00 AM | Sync usage to Adyen for billing |
| Dunning Retries | Daily | 3:00 AM | Process payment retries |
| Trial Reminders | Daily | 9:00 AM | Schedule trial ending emails |
| Monthly Summaries | Monthly | 1st @ 10 AM | Send monthly usage reports |

### Running Cron Jobs

**Development:**
```bash
cd backend
npm run cron

# Or with ts-node
npx ts-node scripts/cron-jobs.ts
```

**Production (Hetzner / PM2):**
```bash
# App server
cd /opt/adproject/backend

# Ensure the cron runner is a separate PM2 process
pm2 start dist/scripts/cron-jobs.js --name backend-cron
pm2 save
```

**Production (Docker):**
```yaml
# docker-compose.yml
services:
  api:
    image: apexmediation-api:latest
    command: node dist/server.js
  
  cron:
    image: apexmediation-api:latest
    command: node dist/scripts/cron-jobs.js
    restart: always
```

**Production (Kubernetes):**
```yaml
# kubernetes/cron-deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: apexmediation-cron
spec:
  replicas: 1
  selector:
    matchLabels:
      app: apexmediation-cron
  template:
    metadata:
      labels:
        app: apexmediation-cron
    spec:
      containers:
      - name: cron
        image: apexmediation-api:latest
        command: ["node", "dist/scripts/cron-jobs.js"]
```

### Manual Cron Execution

```typescript
// Force run specific job
import { usageMeteringService } from './services/billing/UsageMeteringService';
import { dunningManagementService } from './services/billing/DunningManagementService';

// Sync usage to Adyen
await usageMeteringService.syncUsageToAdyen();

// Process dunning retries
await dunningManagementService.processDunningRetries();

// Check usage limits
await usageMeteringService.checkUsageLimits();
```

---

## API Endpoints

### POST /api/usage/track

Record usage from SDK telemetry.

**Request:**
```json
{
  "api_key": "rsk_abc123",
  "metric_type": "impressions",
  "quantity": 1000,
  "metadata": {
    "platform": "iOS",
    "sdk_version": "1.0.0"
  }
}
```

**Response:**
```json
{
  "success": true,
  "recorded": {
    "customer_id": "cus_xxx",
    "metric_type": "impressions",
    "quantity": 1000
  }
}
```

### GET /api/usage/current

Get current billing period usage (authenticated).

**Headers:**
```
Authorization: Bearer <jwt_token>
```

**Response:**
```json
{
  "usage": {
    "customer_id": "cus_xxx",
    "impressions": 850000,
    "api_calls": 75000,
    "data_transfer_gb": 35.2,
    "period_start": "2025-11-01T00:00:00Z",
    "period_end": "2025-12-01T00:00:00Z"
  },
  "overages": {
    "impressions_overage": 0,
    "impressions_overage_cost_cents": 0,
    "total_overage_cost_cents": 0
  }
}
```

### GET /api/usage/analytics

Get usage analytics for dashboard.

**Query Parameters:**
- `days` (optional): Number of days to retrieve (default: 30, max: 365)

**Response:**
```json
{
  "daily_usage": [
    { "date": "2025-11-01", "impressions": 28000, "api_calls": 2500 },
    { "date": "2025-11-02", "impressions": 32000, "api_calls": 2800 }
  ],
  "total_impressions": 850000,
  "total_api_calls": 75000,
  "avg_daily_impressions": 28333
}
```

### GET /api/usage/limits

Get plan limits and usage percentage.

**Response:**
```json
{
  "plan_type": "starter",
  "limits": {
    "impressions": 1000000
  },
  "current_usage": {
    "impressions": 850000
  },
  "percent_used": 85,
  "remaining": {
    "impressions": 150000
  },
  "will_exceed": false
}
```

---

## Troubleshooting

### Issue: Usage not syncing to Adyen

**Check cron job:**
```bash
# Verify cron is running
docker ps | grep cron
kubectl get pods | grep cron

# Check logs
docker logs apexmediation-cron
kubectl logs -f deployment/apexmediation-cron
```

**Manual sync:**
```typescript
import { usageMeteringService } from './services/billing/UsageMeteringService';
await usageMeteringService.syncUsageToAdyen();
```

**Common causes:**
- Adyen API key missing or invalid
- Subscription doesn't have metered price item
- ClickHouse connection failed

### Issue: Dunning not processing retries

**Check pending retries:**
```sql
SELECT * FROM dunning_attempts
WHERE status IN ('pending', 'retrying')
  AND next_retry_at <= NOW()
ORDER BY next_retry_at ASC;
```

**Force retry:**
```typescript
import { dunningManagementService } from './services/billing/DunningManagementService';
await dunningManagementService.processDunningRetries();
```

### Issue: Emails not sending

**Check SMTP configuration (ApexMail + Postfix):**
```bash
echo $SMTP_HOST   # Should be localhost or mail.apexmediation.ee
echo $SMTP_PORT   # Usually 587
echo $SMTP_USER   # Empty for local Postfix
echo $SMTP_PASS   # Empty for local Postfix
```

**Check email queue:**
```sql
-- Pending emails
SELECT * FROM events
WHERE event_type LIKE 'email.%'
  AND processed_at IS NULL
LIMIT 10;

-- Failed emails
SELECT * FROM events
WHERE event_type LIKE 'email.%'
  AND error_message IS NOT NULL
ORDER BY created_at DESC
LIMIT 10;
```

**Check ApexMail queue status:**
```bash
curl -sS "${CONTROL_PLANE_URL}/api/internal/apexmail/stats"
```

### Issue: Customer suspended incorrectly

**Check dunning history:**
```sql
SELECT * FROM dunning_attempts
WHERE customer_id = 'cus_xxx'
ORDER BY created_at DESC;
```

**Manually reactivate:**
```typescript
import { dunningManagementService } from './services/billing/DunningManagementService';

await dunningManagementService.reactivateSubscription(
  'cus_xxx',
  'sub_xxx'
);
```

### Issue: High usage alert spam

**Check alert frequency:**
```sql
SELECT customer_id, COUNT(*) as alert_count
FROM usage_alerts
WHERE created_at >= NOW() - INTERVAL '7 days'
GROUP BY customer_id
HAVING COUNT(*) > 10
ORDER BY alert_count DESC;
```

**Increase alert threshold or reduce frequency in code.**

---

## Monitoring & Metrics

### Key Metrics to Track

**Usage:**
- Total impressions/day
- Customers approaching limits (>80%)
- Overage charges/month

**Payments:**
- Failed payment rate
- Dunning success rate (% recovered)
- Average days to payment recovery

**Emails:**
- Emails sent/day
- Email delivery rate
- Bounce rate
- Open rate (if tracking enabled)

**Cron Jobs:**
- Job execution time
- Job failure rate
- Queue processing lag

### Grafana Dashboard Queries

**Usage Growth:**
```sql
SELECT 
  DATE(recorded_at) as date,
  SUM(CASE WHEN metric_type = 'impressions' THEN quantity ELSE 0 END) as impressions
FROM usage_records
WHERE recorded_at >= NOW() - INTERVAL '30 days'
GROUP BY DATE(recorded_at)
ORDER BY date ASC;
```

**Dunning Effectiveness:**
```sql
SELECT 
  attempt_number,
  COUNT(*) as total,
  COUNT(CASE WHEN status = 'succeeded' THEN 1 END) as recovered,
  (COUNT(CASE WHEN status = 'succeeded' THEN 1 END)::float / COUNT(*)) * 100 as recovery_rate
FROM dunning_attempts
WHERE created_at >= NOW() - INTERVAL '90 days'
GROUP BY attempt_number
ORDER BY attempt_number;
```

---

## Production Checklist

Before going live:

- [ ] SMTP configured for ApexMail (`SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASS`)
- [ ] Adyen API key configured (`APEX_ADYEN_API_KEY`)
- [ ] ClickHouse database created and accessible
- [ ] Database migrations run (`008_sales_automation.sql`)
- [ ] Cron jobs deployed and running
- [ ] Email templates tested (send to yourself)
- [ ] Dunning workflow tested (use Adyen test mode)
- [ ] Usage tracking tested (SDK telemetry endpoint)
- [ ] Monitoring dashboard created (Grafana/Prometheus)
- [ ] Alerting configured (failed payments, cron failures)
- [ ] Documentation reviewed by team

---

**Document Version:** 1.0  
**Last Updated:** 2025-11-03  
**Maintained By:** Sabel Akhoua  
**Next Review:** After first 100 customers
