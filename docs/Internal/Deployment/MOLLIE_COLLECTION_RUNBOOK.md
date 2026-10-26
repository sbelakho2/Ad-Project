# Mollie collection runbook

Purpose: stand up or rotate the Mollie wiring for the monthly lift fee. Apex
collects a fee only on platform-proven lift; collection uses a Mollie payment
link per invoice (or a recurring payment against a stored mandate), and the
webhook fetches the payment from Mollie before recording anything. There is no
usage metering.

## 1. Preflight

| Step | Owner | Evidence |
| --- | --- | --- |
| Mollie account with payouts enabled | Finance | Mollie dashboard shows a verified business |
| Business verification and payout details complete | Finance | Mollie account verification is green |
| Tax settings contain the company vat id | Finance | Mollie tax settings screenshot |

## 2. Credentials

Set these in the deployment environment file (see `infrastructure/README.md`):

- `APEX_MOLLIE_API_KEY` (bearer API key);
- `APEX_MOLLIE_API_BASE` (`https://api.mollie.com`, or a local simulator);
- `APEX_MOLLIE_WEBHOOK_URL` (the public `POST /v1/mollie/webhook` URL);
- `APEX_MOLLIE_REDIRECT_URL` (where the shopper returns after checkout).

There is **no webhook signing key**: Mollie webhooks are unsigned. Rotation is
covered by `MOLLIE_KEYS_ROTATION.md`. Rotate any value that was ever committed,
then verify the webhook and a checkout payment in the test environment before
serving production traffic.

## 3. The monthly invoice

The billing pass reads the signed ledger and bills each kept change from its
pre-registered scope:

1. Run `apex-runner monthly` (the systemd timer runs it on the first of the
   month and bills the month just closed).
2. Each invoice line is re-derived from its proof; the fee is the proven
   lift times the contracted rate, and vat is added on top (`with_vat`).
3. The gross is sent to Mollie as a checkout payment
   (`POST /v2/payments`, `Authorization: Bearer`) with
   `amount: {currency, value}` as a decimal string, `description`,
   `redirectUrl`, `webhookUrl` and `metadata.reference`. The deterministic
   reference is also the `Idempotency-Key`, so a resumed run returns the same
   link.
4. A paid first payment creates a customer and mandate. Later months charge it
   with `POST /v2/payments`, `customerId`, `mandateId` and
   `sequenceType: "recurring"`; a non-paid recurring status falls back to a
   checkout payment with the typed reason recorded.

## 4. Webhooks

Mollie delivers `POST /v1/mollie/webhook` as
`application/x-www-form-urlencoded` with one field: `id=tr_...`.

The route does not trust the body. It fetches `GET /v2/payments/{id}` with the
API key, then validates:

- the fetched id equals the posted id;
- the fetched status is a known Mollie status;
- `metadata.reference` names an invoice or watch reference this deployment
  issued;
- the fetched amount and currency match the expected amount (and
  `metadata.amount` agrees with the fetched amount);
- for a watch payment, the amount equals the tier price quoted to that
  subscriber.

Only a `paid` payment activates a watch or appends a `paid` outcome. The
response is `{"received":true,"id":"tr_...","status":"paid","new":true}`; a
replay answers `"new": false`.

Failure states: malformed form `400 malformed_event`; unknown/forged id
`400 unknown_payment`; mismatched or underpaid payment `400 invalid_payment`;
configured-fetch failures `502 platform_unavailable`; no client
`503 mollie_webhook_not_configured`.

Register the endpoint in the Mollie dashboard and point it at the API domain.

## 5. Verification

1. Post `id=<payment>` and confirm `200` with `new: true`.
2. Confirm the collection log records exactly one `(payment id, status)` row.
3. Re-post the same id and confirm `200` with `new: false` and no second row.
4. Post a forged id and confirm `400 unknown_payment`.
5. Render the invoice document (`render_vat_invoice`) and check the net, vat
   and gross lines.

## 6. Go live

1. Swap the test credentials for live ones and redeploy.
2. Run a small monthly pass and confirm the checkout payment opens.
3. Record the outcome in the production readiness checklist.
