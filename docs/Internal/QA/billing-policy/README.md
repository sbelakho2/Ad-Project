# Billing Policy QA Evidence

Use this folder to collect screenshots, `.eml` exports, or PDFs proving that the updated billing notifications and UI messaging match the `adyen-mandatory-2025-11` policy snapshot.

## Required Artifacts

| Artifact | Trigger / How to Capture | What to Validate | File Name Convention |
| --- | --- | --- | --- |
| Billing preview email | Trigger `email.billing_preview` via staging job or `adyen test clock` five days before due date. | Subject references month + autopay date, copy lists Adyen rails (card/ACH/SEPA) plus Wise manual exception, CTA links to invoice + payment update. | `billing-preview-<YYYYMM>-<account>.png` |
| Payment failed email | Simulate `invoice.payment_failed` via Adyen CLI with declined test card. | Retry cadence text (day 3/5/7), autopay rails list, Wise fallback mention, CTA to update payment method. | `payment-failed-<invoice>.png` |
| Payment retry email | Use `adyen test helpers retry-payment-intent` between attempts. | Attempt counter in subject/body, remaining retries warning, CTA link, manual rails caveat. | `payment-retry-attempt-<N>-<invoice>.png` |
| Payment succeeded after retry email | Complete final retry with successful charge. | Confirms autopay success, lists method charged, notes receipt availability + finance contact. | `payment-succeeded-retry-<invoice>.png` |
| Console billing screenshots | Run staging console at 100% zoom, capture `/billing/settings` + upgrade modal. | Starter free cap text, autopay info card, policy JSON snippet. | `console-billing-<view>.png` |
| Website pricing + signup | Use production/staging site after deploy. | Starter cap + autopay rails copy matches policy wording. | `website-pricing-<date>.png` |

Store raw `.eml` downloads alongside screenshots when possible so we can prove the exact headers delivered by Resend.

## Generate Sample HTML (for quick review)

Run `npm run qa:billing-emails --workspace backend` to regenerate HTML previews under `docs/Internal/QA/billing-policy/samples/`. These files use the exact template builders from `backend/services/email/EmailAutomationService.ts`, so reviewers can open them locally, annotate in Figma, or use them as a reference while waiting for Resend to fire in staging. Replace them with real `.eml` exports once available.

## Submission Checklist

- [ ] All artifacts saved using the naming convention above.
- [ ] Each artifact referenced from `docs/Internal/Deployment/BILLING_POLICY_ROLLOUT.md` Evidence Tracker table.
- [ ] Sensitive data (customer email, invoice IDs) redacted before sharing outside the secure repo.
- [ ] When capturing from Adyen test mode, note the `test_clock` or invoice ID in the filename.

Once all boxes are checked, update the production readiness checklist (`docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md`) under **Starter → Autopay Enforcement QA** to mark "QA evidence captured" as complete.
