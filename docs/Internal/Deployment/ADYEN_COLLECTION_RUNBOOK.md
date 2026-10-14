# Adyen Collection Runbook

_Last updated: 2025-11-24_

Purpose: describe the exact sequence required to accept real payments through Adyen (cards + wallets) **and** to validate the Wise wiring instructions that accompany every invoice. Follow these steps whenever we stand up a new environment or rotate credentials.

---

## 1. Preflight Checklist

| Step | Owner | Evidence |
| --- | --- | --- |
| Create Adyen account & enable EU payouts | Founder / Finance | Screenshot of Adyen dashboard showing verified business | 
| Complete KYC + beneficiary details (Estonian company, Wise payout) | Finance | Adyen "Account verification" card = green | 
| Configure VAT settings (EE VAT ID EE102569407) | Finance | Adyen Tax settings screenshot |
| Link Wise EUR/ USD accounts for payouts | Finance | Adyen Payout settings showing Wise IBAN + ACH details |
| Store `STRIPE_SECRET_KEY` + `STRIPE_WEBHOOK_SECRET` in Infisical | Eng | Infisical entry `backend/STRIPE_SECRET_KEY` |

Once the above is complete you can move to product configuration.

---

## 2. Products & Prices

Adyen should mirror our BYO tiers so usage based fees bill cleanly.

1. **Create Product** `ApexMediation Platform Fee`.
2. **Create Price** per tier using metered billing:
   - `Tier 0` – Type: metered, unit label `EUR of mediated revenue`, price `0` (for completeness).
   - `Tier 1` – Usage type: metered, aggregate `sum`, `2.5%` (0.025 EUR) per EUR of mediated revenue.
   - `Tier 2` – Usage type: metered, aggregate `sum`, `0.02` EUR per EUR.
   - `Tier 3` – Custom negotiated price (create per enterprise deal, usage type metered or flat minimum).
3. Enable **graduated pricing** so that revenue slices map automatically (Starter free, Growth 2.5%, Scale 2.0%). Adyen UI → Price → Add another tier.
4. Record the generated `price_xxx` IDs inside `docs/Internal/Secrets/STRIPE_PRICE_IDS.md` (not committed) and inject them via Infisical.

---

## 3. Metered Usage Sync

Our backend emits usage to Adyen via the `/adyen/usage` job. To finalize:

1. Set env vars in production:
   - `STRIPE_SECRET_KEY`
   - `STRIPE_WEBHOOK_SECRET`
   - `STRIPE_PRICE_GROWTH`, `STRIPE_PRICE_SCALE`, etc. (use the IDs from step 2).
2. Schedule the **`Daily 2 AM: Adyen usage sync`** cron (`backend/scripts/cron-jobs.ts` already wires it).
3. Run a manual sync to confirm connectivity:

```bash
cd backend
STRIPE_SECRET_KEY=sk_test_xxx npm run ts-node src/scripts/backfillStripeUsage.ts -- --dry-run
```

Expect to see log lines grouping mediated revenue per customer and pushing usage records to Adyen.

---

## 4. Webhooks & Portal

1. Run `adyen listen --events invoice.payment_succeeded,invoice.payment_failed --forward-to http://localhost:4000/api/webhooks/adyen`.
2. Copy the signing secret to `STRIPE_WEBHOOK_SECRET` (Infisical + deployment secrets).
3. In production, expose `/api/webhooks/adyen` via the Hetzner reverse proxy (Caddy) on `api.apexmediation.ee`.
4. Enable **Customer Portal** → Settings → Customer portal. Allow customers to update payment methods or download invoices.

Test locally by hitting `POST /api/webhooks/adyen` with a signed payload from the CLI to verify we log status transitions.

---

## 5. Test Mode Dry-Run

1. `adyen customers create --email test+adyen@apexmediation.ee` → grab `cus_test` ID and store in the `users` row.
2. Create a usage record:

```bash
adyen billing/meter-events create \
  --customer=cus_test \
  --event_name=mediated_revenue_eur \
  --value=2500000 \
  --timestamp=$(date +%s)
```

3. Run `adyen invoices create --customer=cus_test --collection_method=charge_automatically`.
4. Finalize and pay the invoice in test mode (`adyen invoices pay INV_ID`).
5. Confirm the backend webhook changed status to `paid` and stored the `stripe_invoice_id`.
6. Download the PDF from console → Billing. Ensure the Wise SEPA + ACH block renders in the email + PDF.

Document screenshots in `docs/Internal/QA/adyen-dry-run/`.

---

## 6. Go-Live Switch

1. Replace test keys with live keys in Infisical and redeploy backend.
2. Toggle Adyen dashboard to **Live** mode, re-run a small usage sync to confirm events go through.
3. Process a €0 test invoice (use manual invoice with 0 amount) to verify email deliverability.
4. Update `docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md` checkboxes for the Adyen section once the above evidence exists.

---

## 7. Wise Invoice Validation Loop

After each monthly invoice batch:

1. Pick one customer who pays via wire.
2. Open console → Billing → Invoice preview. Confirm IBAN + ACH instructions match `backend/src/config/banking.ts`.
3. Forward the invoice email to finance@ for manual inspection.
4. Track results in `notion://Billing QA` (column "Wise copy validated").

Set a recurring calendar reminder for the 2nd of every month to perform this QA.

---

By following this runbook we can flip Adyen to production in <30 minutes and keep the Wise instructions consistent across console and email surfaces.
