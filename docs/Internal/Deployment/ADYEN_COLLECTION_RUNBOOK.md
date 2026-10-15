# Adyen collection runbook

Purpose: stand up or rotate the Adyen wiring for the monthly lift fee. Apex
collects a fee only on platform-proven lift; the collection is a Checkout
payment link per invoice, and the webhook turns collection state into ledger
records. There is no usage metering.

## 1. Preflight

| Step | Owner | Evidence |
| --- | --- | --- |
| Adyen account with payouts enabled | Finance | Adyen dashboard shows a verified business |
| Business verification and payout details complete | Finance | Adyen account verification is green |
| Tax settings contain the company vat id | Finance | Adyen tax settings screenshot |

## 2. Credentials

Set these in the deployment environment file (see `infrastructure/README.md`):

- `APEX_ADYEN_API_KEY`, `APEX_ADYEN_MERCHANT_ACCOUNT`;
- `APEX_ADYEN_HMAC_KEY` (the webhook signing key);
- `APEX_ADYEN_CHECKOUT_BASE` (no version, for example the test host).

Rotate any value that was ever committed, then re-verify the webhook in the
test environment before serving production traffic.

## 3. The monthly invoice

The billing pass reads the signed ledger and bills each kept change from its
pre-registered scope:

1. Run `apex-runner monthly` (the systemd timer runs it on the first of the
   month and bills the month just closed).
2. Each invoice line is re-derived from its proof; the fee is the proven
   lift times the contracted rate, and vat is added on top (`with_vat`).
3. The gross is sent to Adyen as a Checkout payment link
   (`POST /v71/paymentLinks`), with the deterministic reference as the
   `Idempotency-Key`, so a resumed run returns the same link.

## 4. Webhooks

Adyen standard webhooks arrive at `POST /v1/adyen/webhook` as a
`notificationItems` envelope. Each item carries its own signature in
`additionalData.hmacSignature`, which the route verifies over the eight signed
fields before accepting. The response is `{"received":true,"items":n}`.

Register the endpoint in the Adyen dashboard and point it at the API domain.
A delivery with a bad signature is `400 invalid_signature`; a malformed
envelope is `400 malformed_event`; an unconfigured key is
`503 adyen_webhook_not_configured`.

## 5. Verification

1. Post a signed test notification and confirm a `200` with the item count.
2. Confirm the ledger records the collection transition.
3. Render the invoice document (`render_vat_invoice`) and check the net, vat
   and gross lines.

## 6. Go live

1. Swap the test credentials for live ones and redeploy.
2. Re-run a small monthly pass and confirm the payment link opens.
3. Record the outcome in the production readiness checklist.
