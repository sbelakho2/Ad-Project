# Billing Policy QA Evidence

Use this folder to collect screenshots, `.eml` exports, or PDFs proving that the updated billing notifications and UI messaging match the `mollie-mandatory-2025-11` policy snapshot.

## Required Artifacts

| Artifact | Trigger / How to Capture | What to Validate | File Name Convention |
| --- | --- | --- | --- |
| Billing preview email | Trigger the fee preview from the staging runner before the invoice is issued. | Copy matches the `fee_preview` template in `rust/crates/apex-notify/src/template.rs` (subject names the month, the amount, one action). | `billing-preview-<YYYYMM>-<account>.png` |
| Payment failed email | Record a refused authorisation event through the collection path. | Copy matches the `payment_failed` template in `rust/crates/apex-notify/src/template.rs`; it states that no payment is confirmed and shows a retry date only when one is scheduled. | `payment-failed-<invoice>.png` |
| Payment retry email | Record a scheduled retry in the finance boundary; the runner sends the retry message on its next pass. | Copy matches the `payment_retry` template in `rust/crates/apex-notify/src/template.rs`; a retry date renders only when a real retry exists. | `payment-retry-attempt-<N>-<invoice>.png` |
| Payment succeeded after retry email | Record a successful authorisation event through the collection path. | Copy matches the `payment_received` template in `rust/crates/apex-notify/src/template.rs`; a partial payment states the received amount and the remaining balance. | `payment-succeeded-retry-<invoice>.png` |
| Console billing screenshots | Run staging console at 100% zoom, capture `/billing/settings` + upgrade modal. | Starter free cap text, autopay info card, policy JSON snippet. | `console-billing-<view>.png` |
| Website pricing + signup | Use production/staging site after deploy. | Starter cap + autopay rails copy matches policy wording. | `website-pricing-<date>.png` |

Store raw `.eml` downloads alongside screenshots when possible so we can prove the exact headers delivered by Resend.

## Generate Sample HTML (for quick review)

The HTML previews under `docs/Internal/QA/billing-policy/samples/` are thin pointers to the apex-notify template catalog (`rust/crates/apex-notify/src/template.rs`), which carries the frozen copy of the design document (section 06, pages 93 to 95). Each sample reproduces one template's subject and body copy verbatim; live messages are rendered from that catalog by the runner and their delivery states are recorded by the notification state store. Replace the samples with real `.eml` exports once available.

## Submission Checklist

- [ ] All artifacts saved using the naming convention above.
- [ ] Each artifact referenced from `docs/Internal/Deployment/BILLING_POLICY_ROLLOUT.md` Evidence Tracker table.
- [ ] Sensitive data (customer email, invoice IDs) redacted before sharing outside the secure repo.
- [ ] When capturing from Mollie test mode, note the `test_clock` or invoice ID in the filename.

Once all boxes are checked, update the production readiness checklist (`docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md`) under **Starter → Autopay Enforcement QA** to mark "QA evidence captured" as complete.
