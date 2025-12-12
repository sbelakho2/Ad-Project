# Billing Platform Deployment Checklist

## Pre-Deployment Verification

### Code Review
- [x] All TypeScript files compile without errors
- [x] No breaking changes to existing functionality
- [x] All routes follow existing authentication patterns
- [x] Error handling implemented consistently
- [x] Logging added for all critical operations

### Database
- [ ] Review migration scripts (017, 018)
- [ ] Backup production database before migration
- [ ] Test migrations on staging environment
- [ ] Verify rollback procedures documented

### Environment Variables
- [ ] Backend `.env` configured:
  - [ ] `BILLING_ENABLED=true`
  - [ ] `STRIPE_SECRET_KEY=sk_live_...` (production key)
  - [ ] `STRIPE_WEBHOOK_SECRET=whsec_...`
  - [ ] `DATABASE_URL` points to production
- [ ] Console `.env.local` configured:
  - [ ] `NEXT_PUBLIC_BILLING_ENABLED=true`
  - [ ] `NEXT_PUBLIC_API_URL` points to production API

### Security Review
- [x] Authentication required on all billing endpoints
- [x] RBAC enforcement for admin operations
- [x] CSRF protection enabled
- [x] Stripe webhook signature verification implemented
- [ ] Security audit completed by team
- [ ] Penetration testing performed

---

## Deployment Steps

### Step 1: Database Migration
```bash
# Backup database
pg_dump $DATABASE_URL > backup_$(date +%Y%m%d_%H%M%S).sql

# Run billing migrations
cd /home/aaron/IdeaProjects/Ad-Project
./scripts/run-billing-migrations.sh

# Verify tables created
psql $DATABASE_URL -c "\dt billing_*"
psql $DATABASE_URL -c "\dt stripe_webhook_events"
```
- [ ] Migrations executed successfully
- [ ] Tables created with correct schema
- [ ] Indexes created on all required columns

### Step 2: Stripe Configuration
```bash
# Go to Stripe Dashboard > Developers > Webhooks
# Click "Add endpoint"
```
1. [ ] Set webhook URL: `https://yourdomain.com/api/v1/webhooks/stripe`
2. [ ] Select events:
   - [ ] `invoice.created`
   - [ ] `invoice.finalized`
   - [ ] `invoice.payment_succeeded`
   - [ ] `invoice.payment_failed`
   - [ ] `charge.refunded`
   - [ ] `customer.subscription.updated`
   - [ ] `customer.subscription.deleted`
3. [ ] Copy webhook signing secret to `STRIPE_WEBHOOK_SECRET`
4. [ ] Test webhook delivery with "Send test webhook"

### Step 3: Backend Deployment
```bash
cd backend
npm run build
pm2 restart apexmediation-backend
```
- [ ] Backend builds successfully
- [ ] Service starts without errors
- [ ] Health check endpoint responds: `curl https://yourdomain.com/api/v1/health`
- [ ] Feature flags endpoint returns billing enabled: `curl https://yourdomain.com/api/v1/meta/features`

### Step 4: Console Deployment
```bash
cd console
npm run build
pm2 restart apexmediation-console
```
- [ ] Console builds successfully
- [ ] Service starts without errors
- [ ] Homepage loads correctly
- [ ] Navigation shows billing menu item

---

## Post-Deployment Testing

### Smoke Tests

#### 1. Feature Flags
```bash
curl https://yourdomain.com/api/v1/meta/features
# Expected: {"billingEnabled":true,...}
```
- [ ] Feature flags endpoint returns 200 OK
- [ ] `billingEnabled` is `true`

#### 2. Usage Endpoint (Authenticated)
```bash
curl -H "Cookie: auth_token=..." https://yourdomain.com/api/v1/billing/usage/current
```
- [ ] Returns 200 OK with usage data
- [ ] Contains `current_period`, `overages`, `subscription` fields
- [ ] Returns 401 without authentication

#### 3. Invoices List (Authenticated)
```bash
curl -H "Cookie: auth_token=..." https://yourdomain.com/api/v1/billing/invoices
```
- [ ] Returns 200 OK with invoices array
- [ ] Pagination object included
- [ ] Returns 401 without authentication

#### 4. Webhook Handler
```bash
# Send test webhook from Stripe Dashboard
```
- [ ] Webhook received successfully
- [ ] Event logged to `stripe_webhook_events` table
- [ ] Signature verification passes
- [ ] Duplicate events rejected

#### 5. Console UI
- [ ] Navigate to `/billing/usage`
- [ ] Usage metrics display correctly
- [ ] Navigate to `/billing/invoices`
- [ ] Invoice list loads with pagination
- [ ] Click invoice to view detail page
- [ ] Download PDF button works

### Integration Tests

#### Usage Tracking Flow
1. [ ] Generate test traffic to app
2. [ ] Wait 5 minutes for usage aggregation
3. [ ] Check `/billing/usage/current` shows updated numbers
4. [ ] Verify Postgres analytics replica (read-model) has usage records
5. [ ] Verify PostgreSQL `usage_records` table updated

#### Invoice Generation Flow
1. [ ] Trigger Stripe subscription billing (or wait for billing cycle)
2. [ ] Verify `invoice.created` webhook received
3. [ ] Check `invoices` table for new record
4. [ ] Verify invoice appears in console UI
5. [ ] Download PDF and verify content

#### Reconciliation Flow (Admin Only)
```bash
# Generate idempotency key
IDEM_KEY="test_$(date +%s)"

# Trigger reconciliation
curl -X POST \
  -H "Cookie: auth_token=..." \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: $IDEM_KEY" \
  https://yourdomain.com/api/v1/billing/reconcile
```
- [ ] Returns 200 OK with reconciliation result
- [ ] Discrepancies logged to `billing_audit`
- [ ] Duplicate request returns cached result
- [ ] Returns 403 for non-admin users

---

## Monitoring Setup

### Metrics to Track
- [ ] `/billing/usage/current` latency (p95 < 500ms)
- [ ] `/billing/invoices` latency (p95 < 1s)
- [ ] Webhook processing success rate (> 99%)
- [ ] Reconciliation discrepancy rate (< 1%)
- [ ] Invoice PDF generation time (< 2s)

### Alerts to Configure
- [ ] Webhook signature verification failures (> 5/hour)
- [ ] Reconciliation discrepancies (> $1000 total)
- [ ] Failed invoice payments (immediate alert)
- [ ] Database migration errors
- [ ] API endpoint errors (> 10/minute)

### Log Monitoring
```bash
# Watch backend logs for errors
tail -f backend/logs/error.log | grep billing

# Watch for webhook events
psql $DATABASE_URL -c "SELECT * FROM stripe_webhook_events ORDER BY created_at DESC LIMIT 10;"

# Check audit trail
psql $DATABASE_URL -c "SELECT * FROM billing_audit WHERE event_type='error' ORDER BY created_at DESC LIMIT 10;"
```

---

## Rollback Procedures

### If Issues Detected

#### Option 1: Disable Feature Flags
```bash
# Backend
export BILLING_ENABLED=false
pm2 restart apexmediation-backend

# Console
export NEXT_PUBLIC_BILLING_ENABLED=false
pm2 restart apexmediation-console
```
- [ ] Billing menu item disappears
- [ ] Billing endpoints return 503 Service Unavailable
- [ ] Existing functionality unaffected

#### Option 2: Database Rollback
```sql
-- Drop new tables (CAUTION: Data loss!)
DROP TABLE IF EXISTS stripe_webhook_events CASCADE;
DROP TABLE IF EXISTS events CASCADE;
DROP TABLE IF EXISTS usage_alerts CASCADE;
DROP TABLE IF EXISTS billing_idempotency CASCADE;
DROP TABLE IF EXISTS billing_audit CASCADE;
```
- [ ] Backup created before rollback
- [ ] Tables dropped successfully
- [ ] Restore from backup if needed

#### Option 3: Full Rollback
```bash
# Restore database backup
psql $DATABASE_URL < backup_YYYYMMDD_HHMMSS.sql

# Deploy previous backend version
git checkout <previous-commit>
cd backend && npm run build && pm2 restart apexmediation-backend

# Deploy previous console version
cd console && npm run build && pm2 restart apexmediation-console
```

---

## Success Criteria

### Functional Requirements
- [x] Users can view current usage with overage detection
- [x] Users can list and filter invoices
- [x] Users can view invoice details with line items
- [x] Users can download invoice PDFs
- [x] Admins can trigger reconciliation
- [x] Stripe webhooks processed automatically
- [x] Feature flags control billing visibility

### Non-Functional Requirements
- [ ] API latency < 500ms (p95)
- [ ] Webhook processing > 99% success rate
- [ ] Zero downtime during deployment
- [ ] No breaking changes to existing features
- [ ] All security requirements met

### User Experience
- [ ] Billing pages load in < 2 seconds
- [ ] PDF downloads complete in < 3 seconds
- [ ] Error messages are user-friendly
- [ ] Mobile responsive design works correctly
- [ ] Navigation is intuitive

---

## Stakeholder Sign-Off

### Development Team
- [ ] Code review completed by: _______________
- [ ] Security review completed by: _______________
- [ ] QA testing completed by: _______________

### Product Team
- [ ] Product manager approval: _______________
- [ ] UX/UI approval: _______________

### Operations Team
- [ ] Infrastructure ready: _______________
- [ ] Monitoring configured: _______________
- [ ] Runbook documented: _______________

### Finance Team
- [ ] Stripe integration verified: _______________
- [ ] Reconciliation process approved: _______________

---

## Post-Launch

### Week 1
- [ ] Monitor error rates daily
- [ ] Check webhook processing logs
- [ ] Review first reconciliation results
- [ ] Gather user feedback
- [ ] Document any issues in runbook

### Week 2
- [ ] Analyze usage patterns
- [ ] Review API performance metrics
- [ ] Optimize slow queries if needed
- [ ] Update documentation based on findings

### Month 1
- [ ] Full retrospective with team
- [ ] Plan Phase 2 enhancements
- [ ] Update pricing models if needed
- [ ] Celebrate successful launch! 🎉

---

## Support Resources

### Documentation
- [Billing Implementation Summary](./BILLING_IMPLEMENTATION_SUMMARY.md)
- [Billing README](./console/BILLING_README.md)
- [Backend API Documentation](./backend/openapi.yaml)

### Team Contacts
- Backend Lead: _______________
- Frontend Lead: _______________
- DevOps: _______________
- Finance: _______________

### Emergency Contacts
- On-Call Engineer: _______________
- Engineering Manager: _______________
- CTO: _______________

---

**Deployment Date:** _______________  
**Deployed By:** _______________  
**Status:** _______________ (Pending/In Progress/Complete)
