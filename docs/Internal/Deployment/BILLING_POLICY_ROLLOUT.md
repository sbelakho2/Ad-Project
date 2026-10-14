# Adyen-First Billing Policy Rollout Plan

_Date:_ 2025-11-24  
_Owner:_ Billing/Platform  
_Scope:_ Align backend, console, website, billing services, and QA workflows to the Starter (free) → paid tiers policy.

## Policy Snapshot
1. **Starter (≤ $10k mediated revenue / mo)**
   - No payment method required.
   - Access: SDK integration, dashboards/metrics, VRA, debugger, Migration Studio within Starter caps.
   - Messaging: “Free up to $10k/month. No credit card. No bank. Plug in, see if you like it.”
2. **Growth / Scale / Enterprise**
   - Triggered automatically once revenue exceeds $10k or user clicks **Upgrade**.
   - Require payment method (card, ACH, SEPA; Enterprise may request invoice/wire).
   - Default behavior is auto-charge at end of each billing period; Enterprise can switch to manual invoices/net terms.
3. **Billing Cycle**
   - Monthly invoice per account describing mediated revenue by app/tier and Apex platform fee % (2.5%, 2%, custom).
   - Dashboard + email pre-notification before auto-charge; autopay executes on due date.
   - UI highlights reconciled revenue (VRA), adjustments/credits, and explicitly states “We don’t touch your ad payouts.”

## Task Matrix

| Area | Owner | Key Tasks |
| --- | --- | --- |
| Backend API | Billing Platform | Expand `billingPolicy` schema to encode Starter vs paid behavior, autopay defaults, and upgrade triggers; expose new metadata via `/api/v1/billing/policy` and `billing` routes; guard upgrade endpoints with payment method requirements; emit notifications (email/webhooks) before auto-charge. |
| Billing Services | Finance Ops | Update invoice generation service to break down revenue by tier/app, compute fees, and attach transparency payload; ensure autopay schedule + retries align with policy; persist adjustment notes. |
| Console UI | Console Team | Starter screens highlight “No payment method” until upgrade; upgrade flow collects payment method choices; billing dashboard shows monthly fee preview, reconciled VRA data, adjustments, and autopay messaging; admin override toggles for Enterprise invoice terms. |
| Website / Docs | Growth | Refresh marketing copy to emphasize free Starter, autopay defaults, and “we only bill platform fee” transparency; update pricing + FAQ + onboarding docs; mention upgrade triggers and accepted payment rails. |
| Quality & Monitoring | QA | Add tests (backend + console) covering policy transitions, autopay notifications, invoice transparency; update runbooks/checklists with verification steps; ensure observability emits events for autopay/delinquency states. |

## Execution Order
1. **Backend foundation** – extend config/models/tests, emit new data to consumers.
2. **Billing services & notifications** – compute fees, generate invoices, send pre-charge messages.
3. **Console UI** – reflect Starter messaging, upgrade flow, invoice transparency.
4. **Website/docs** – align marketing + support materials.
5. **Quality suite** – new tests, monitoring hooks, runbook updates.

Each phase should land with feature flags (`billingPolicyVersion`) to stage changes safely.

## Progress Log
- **2025-11-24:** Website pricing + signup copy now mirror `adyen-mandatory-2025-11` messaging (Starter free cap, autopay rails, Enterprise exception). Website lint/tests + production build executed post-change; screenshots pending capture for QA evidence folder.
- **2025-11-24:** Customer-facing billing docs (`docs/Customer-Facing/Billing-Compliance/pricing.md`, `docs/Customer-Facing/Compliance/Invoicing-Payments.md`) now call out Starter no-card policy, autopay requirements for paid tiers, Adyen retry cadence, and the manual Wise rails as finance-approved exceptions. Next evidence: upload pricing + signup screenshots and link them in this log.
- **2025-11-24:** Postfix/ApexMail billing notifications updated (`backend/services/email/EmailAutomationService.ts`) to include the 5-day billing preview email and Adyen autopay receipt/failed retry messaging per `billingPolicy.billingCycle.notifications`. Pending evidence: capture sample emails in `docs/Internal/QA/billing-policy/` once staging jobs fire.
- **2025-11-24:** Production readiness checklist reformatted into explicit checkboxes (`docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md`) with the QA evidence gate pointing at `docs/Internal/QA/billing-policy/` so rollout blockers stay visible.
- **2025-11-25:** Extracted billing-email builders (`backend/services/email/billingEmailBuilders.ts`) + `npm run qa:billing-emails --workspace backend` to render HTML previews under `docs/Internal/QA/billing-policy/samples/` while waiting for live Postfix email captures.

## Evidence Tracker

| Flow | Event / Template | Policy Clause | Evidence |
| --- | --- | --- | --- |
| Billing preview | `email.billing_preview` (`sendBillingPreviewEmail`) | `billingPolicy.billingCycle.notifications.preview` (5-day heads up, Adyen rails, Wise fallback) | Preview HTML: `docs/Internal/QA/billing-policy/samples/billing-preview-sample.html` (need live `.eml` + screenshot) |
| Payment failed | `email.payment_failed` (`sendPaymentFailedEmail`) | Retry cadence + autopay rails from `billingPolicy.billingCycle.retries` | Preview HTML: `docs/Internal/QA/billing-policy/samples/payment-failed-sample.html` (need Postfix evidence) |
| Payment retry | `email.payment_retry` (`sendPaymentRetryEmail`) | Final warning + remaining attempts notice per policy | Preview HTML: `docs/Internal/QA/billing-policy/samples/payment-retry-sample.html` (need attempt-2/3 email capture) |
| Payment succeeded after retry | `email.payment_succeeded_after_retry` (`sendPaymentSucceededEmail`) | Receipt + rail confirmation text | Preview HTML: `docs/Internal/QA/billing-policy/samples/payment-succeeded-sample.html` (need real receipt email) |
| Website pricing & signup | Console + marketing captures | Starter promise + autopay copy (`adyen-mandatory-2025-11`) | _Pending:_ screenshots per `docs/Internal/QA/billing-policy/README.md` |

Reference: drop raw evidence files into `docs/Internal/QA/billing-policy/` and update the table links once uploaded. For interim reviews, run `npm run qa:billing-emails --workspace backend` to refresh the HTML previews under `docs/Internal/QA/billing-policy/samples/` (they use the exact email builders).
