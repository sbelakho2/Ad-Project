# Billing Module

## Overview

The billing module provides usage tracking, invoice management, and reconciliation features for the ApexMediation console.

> This module is live in production; anything marked as "Planned" lives behind feature flags and must not be promised externally until the referenced FIX is complete.

## Feature Parity Snapshot — 2025-12-12
| Capability | Surface | Status | Notes |
| --- | --- | --- | --- |
| Usage tracking & overage detection | `/billing/usage`, `GET /api/v1/billing/usage/current` | ✅ Shipped | Data sourced from Postgres analytics replicas with hourly cache invalidation. |
| Invoice management & PDFs | `/billing/invoices`, `/billing/invoices/[id]`, `GET /api/v1/billing/invoices*` | ✅ Shipped | Detail pages + PDF downloads in console, Stripe webhook sync in backend. |
| Admin reconciliation | `/admin/billing`, `POST /api/v1/billing/reconcile` | ✅ Shipped | Requires admin role and acknowledgement prompt. |
| **Stripe Webhook Integration** | `POST /api/v1/webhooks/stripe` | ✅ Shipped (2025-12-12) | Raw body parsing for signature verification; 12+ event types tested. |
| Migration Assistant (beta) | `/billing/settings` (flag `NEXT_PUBLIC_ENABLE_BILLING_MIGRATION`), `POST /api/v1/billing/migration/request` | ✅ Pilot | Sends sandbox/production migration context to ops with request IDs and char-count validation. |
| Payment method management | `/billing/payment-methods` (flagged), `POST /api/v1/billing/payment-methods/*` | ⏳ Planned (FIX-03-116 follow-up) | UI/API scaffolding complete; awaiting Stripe customer portal wiring + QA. |
| Dunning management & retries | Console notifications + Stripe webhooks | ⏳ Planned (FIX-03-116 follow-up) | Playbooks documented; automation shipping under FIX-03-154. |
| Usage forecasting & budgeting | `/billing/usage` projections | ⏳ Planned | Depends on analytics forecasting models (FIX-05 alignment). |

## Features (Shipped)

### 1. Usage Tracking (`/billing/usage`)
- Real-time usage metrics (impressions, API calls, data transfer)
- Visual progress bars showing plan limit consumption
- Overage detection and cost calculation
- Support for multiple plan types (Starter, Growth, Scale, Enterprise)

### 2. Invoice Management (`/billing/invoices`)
- Paginated invoice list with filtering by status
- Individual invoice detail pages
- PDF invoice download
- Line item breakdown
- Payment status tracking

### 3. Feature Flags
- Billing features can be enabled/disabled via `NEXT_PUBLIC_BILLING_ENABLED` environment variable
- Navigation menu automatically shows/hides billing link based on flag
- Backend feature flags exposed via `/api/v1/meta/features` endpoint

### 4. Reconciliation (Admin Only)
- Compare internal usage records with Stripe billing data
- Identify discrepancies with configurable tolerance thresholds
- Idempotent reconciliation operations
- Audit trail for all reconciliation activities

### 5. Migration Assistant (Feature Flag)
- Appears inside `/billing/settings` when `NEXT_PUBLIC_ENABLE_BILLING_MIGRATION=true`.
- Lets publishers submit sandbox vs production cutover context with a 20-character minimum.
- Calls `POST /api/v1/billing/migration/request` (returns `{ requestId, status, submittedAt, channel, notesPreview }`).
- Requires the backend route deployed (or MSW handler) before enabling the flag outside local/staging to avoid 404s.

## Upcoming Enhancements (Behind Feature Flags)

### Payment Method Management (`/billing/payment-methods`)
- Entry point hidden unless `NEXT_PUBLIC_ENABLE_BILLING_PM` is true and backend exposes `/api/v1/billing/payment-methods`.
- Allows customers to view the active funding source, rotate cards/bank accounts through Stripe’s Setup Intents, and mark backup methods.
- Integrates with the payout confirmation modal patterns introduced in FIX-03-112 so sensitive actions require a confirmation keyword.
- Backend dependencies: Stripe customer portal enablement + audit logging (#FIX-01-212).

### Dunning & Collections
- Reuses the admin reconciliation audit trail to surface delinquent subscriptions inside `/billing/usage` and `/admin/billing`.
- Automates retry schedules (Day 0/3/7) with localized email/SaaS notifications and escalates to Slack via `billing_audit` triggers.
- Each borrower CTA links to the invoice detail view with contextual messaging so support teams can act quickly.
- Backend dependencies: Stripe event `invoice.payment_failed` handlers + notification templates (FIX-05-041).

## API Endpoints

### Usage
- `GET /api/v1/billing/usage/current` - Get current period usage and overages

### Invoices
- `GET /api/v1/billing/invoices` - List invoices with pagination and filters
- `GET /api/v1/billing/invoices/:id` - Get single invoice
- `GET /api/v1/billing/invoices/:id/pdf` - Download invoice PDF

### Reconciliation
- `POST /api/v1/billing/reconcile` - Trigger reconciliation (admin only)

### Migration
- `POST /api/v1/billing/migration/request` - Queue migration assistant tickets (requires `BILLING_ENABLED=true`)

### Feature Flags
- `GET /api/v1/meta/features` - Get feature flags (public)

## Environment Variables

### Backend
```bash
BILLING_ENABLED=true
STRIPE_SECRET_KEY=sk_test_...
STRIPE_WEBHOOK_SECRET=whsec_...
```

### Console
```bash
NEXT_PUBLIC_BILLING_ENABLED=true
NEXT_PUBLIC_API_URL=http://localhost:8080/api/v1
```

## Architecture

### Frontend (`console/src`)
```
app/billing/
  layout.tsx              # Sub-navigation for billing pages
  page.tsx                # Root redirect to usage page
  usage/
    page.tsx              # Usage tracking page
  invoices/
    page.tsx              # Invoice list page
    [id]/
      page.tsx            # Invoice detail page

lib/
  billing.ts              # Billing API client
  __tests__/
    billing.test.ts       # API client tests

components/
  Navigation.tsx          # Updated with billing link
```

### Backend (`backend/src`)
```
routes/
  billing.routes.ts       # Billing API routes
  webhooks.routes.ts      # Stripe webhook handler
  meta.routes.ts          # Feature flag endpoint

controllers/
  billing.controller.ts   # Billing business logic

services/
  billing/
    UsageMeteringService.ts  # Usage tracking
  invoiceService.ts          # Invoice management
  reconciliationService.ts   # Reconciliation logic

utils/
  featureFlags.ts         # Feature flag utilities
```

## Database Schema

### Tables Created
- `billing_audit` - Audit trail for all billing operations
- `billing_idempotency` - Idempotency key tracking for reconciliation
- `usage_alerts` - Usage threshold alerts
- `stripe_webhook_events` - Webhook event deduplication
- `events` - Async event queue

### Existing Tables Used
- `subscriptions` - Customer subscription details
- `usage_records` - Usage tracking data
- `invoices` - Invoice records

## Usage Examples

### Fetch Current Usage
```typescript
import { getCurrentUsage } from '@/lib/billing'

const usage = await getCurrentUsage()
console.log(usage.current_period.impressions)
console.log(usage.overages.total_overage_cost)
```

### List Invoices
```typescript
import { listInvoices } from '@/lib/billing'

const { invoices, pagination } = await listInvoices({
  page: 1,
  limit: 20,
  status: 'paid',
})
```

### Download Invoice PDF
```typescript
import { downloadInvoicePDF } from '@/lib/billing'

const blobUrl = await downloadInvoicePDF('inv_123')
window.open(blobUrl, '_blank')
```

### Trigger Reconciliation (Admin)
```typescript
import { reconcileBilling } from '@/lib/billing'

const result = await reconcileBilling('idem_key_123')
console.log(result.discrepancies)
```

## Testing

### Run Unit Tests
```bash
cd console
npm test -- billing.test.ts
```

### Run Backend Tests
```bash
cd backend
npm test -- billing.controller.test.ts
```

### Integration Tests
```bash
cd backend
npm test -- integration/billing.test.ts
```

## Stripe Webhook Configuration

### Setup (2025-12-12 Update)

1. Create webhook endpoint in Stripe Dashboard
2. Point to: `https://yourdomain.com/api/v1/webhooks/stripe`
3. Select events:
   - `customer.created`
   - `customer.updated`
   - `payment_method.attached`
   - `invoice.created`
   - `invoice.finalized`
   - `invoice.paid`
   - `invoice.payment_succeeded`
   - `invoice.payment_failed`
   - `charge.succeeded`
   - `charge.refunded`
   - `payment_intent.created`
   - `payment_intent.succeeded`
   - `customer.subscription.created`
   - `customer.subscription.updated`
   - `customer.subscription.deleted`
   - `checkout.session.completed`
4. Copy webhook signing secret to `STRIPE_WEBHOOK_SECRET` env var

### Technical Implementation Note

Stripe webhooks require the raw request body for signature verification. The backend implements this with:

```typescript
// In backend/src/index.ts - BEFORE express.json()
app.use('/api/v1/webhooks/stripe', express.raw({ type: 'application/json' }));
app.use(express.json()); // After raw body route
```

This ensures the Stripe webhook endpoint receives the unmodified body buffer for HMAC signature verification.

### Tested Event Types (2025-12-12)
All 12 event types tested and returning 200 OK:
- `customer.created`, `customer.updated`
- `payment_method.attached`
- `invoice.created`, `invoice.finalized`, `invoice.paid`, `invoice.payment_succeeded`
- `charge.succeeded`, `payment_intent.created`, `payment_intent.succeeded`
- `customer.subscription.created`, `checkout.session.completed`

## Security

- All billing endpoints require authentication via JWT cookies
- Reconciliation endpoint requires admin role
- Webhook signature verification prevents unauthorized events
- Idempotency keys prevent duplicate processing
- CSRF tokens required for mutating operations
- ETag caching for PDF downloads reduces bandwidth

## Performance

- Usage data cached for 1 hour
- Invoice PDFs cached with ETag headers (304 Not Modified support)
- Pagination limits: 1-100 invoices per page
- Database indexes on: customer_id, status, created_at, stripe_event_id

## Monitoring

- All billing operations logged to `billing_audit` table
- Webhook events tracked in `stripe_webhook_events`
- Reconciliation discrepancies logged with alert thresholds
- Usage alerts emitted to `usage_alerts` table

## Troubleshooting

### Billing features not appearing
- Check `NEXT_PUBLIC_BILLING_ENABLED` is set to `true`
- Verify backend `BILLING_ENABLED` environment variable
- Check `/api/v1/meta/features` returns `billingEnabled: true`

### Usage data not updating
- Verify Postgres analytics replica connectivity (DATABASE_URL + REPLICA_DATABASE_URL)
- Check `usage_records` table for recent entries
- Review cron job logs for usage sync tasks

### Invoices not syncing from Stripe
- Verify `STRIPE_SECRET_KEY` is correct
- Check webhook events are being received
- Review `stripe_webhook_events` table for failed events
- Check webhook signature verification

### Reconciliation failing
- Ensure Stripe API key has permissions for `subscriptionItems.usage.listUsageRecordSummaries`
- Check tolerance threshold configuration (default 0.5%)
- Review `billing_audit` for reconciliation errors
- Verify idempotency key is unique

## Delivery Roadmap

| Phase | Scope | Status |
| --- | --- | --- |
| 1 | Usage tracking, invoices, PDFs, Stripe webhook ingestion, reconciliation service | ✅ Complete |
| 2 | Payment method management, subscription upgrades/downgrades, usage forecasting/budgeting, multi-currency, tax integration | 🚧 In progress — align with FIX-03-154 & FIX-05-020. |
| 3 | Dunning workflows, credit/refund tooling, custom billing schedules, usage-based pricing tiers, analytics dashboard | 📝 Planned — requires data pipeline maturity (FIX-06) before scheduling. |

> For any roadmap item marked 🚧 or 📝, reference `docs/Internal/Development/FIXES.md` before communicating timelines. Update this table whenever scope or status changes.

## Support

For issues or questions:
1. Check backend logs: `backend/logs/`
2. Review audit trail: `billing_audit` table
3. Check Stripe Dashboard for payment status
4. Contact platform team via support channel
