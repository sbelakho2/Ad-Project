# ADR-SB-06 — Billing Model & SaaS Fee Structure

## Status
Accepted — 2025-11-19

## Context
Part 0 requires semantic versioning per surface and clarity around billing. We must lock the fee models (SaaS, usage, optional revenue share) and document how invoicing interacts with BYO payouts.

## Decision
- **Fee Types:**
  1. **Platform SaaS Fee** — fixed monthly per active org tier.
  2. **Usage Fee** — per auction request or active placement, tracked in `usage_events`.
  3. **Optional % Net** — negotiated revenue share recorded in `billing_plans` but never touching network payouts.
- **FX Normalization:** ECB daily rates ingested and cached for invoice generation (see `backend/src/services/billing/FxRateService.ts`).
- **Automation:** Adyen Billing handles collection/dunning; `backend/scripts/cron-jobs.ts` syncs usage nightly.
- **Audit Trail:** `billing_audit` table + document retention (7 years) provide traceability.
- **Versioning:** Pricing changes require new `billing_plans.version` entries and release notes referencing this ADR.

## Consequences
- Billing remains transparent and separable from ad network payouts, satisfying BYO constraints.
- Finance/legal reference this ADR when updating pricing or contracts.
- Any deviation (e.g., custodial payouts) demands a new ADR + governance review.
