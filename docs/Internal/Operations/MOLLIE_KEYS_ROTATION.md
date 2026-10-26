# Mollie keys rotation

Rotate the Mollie credentials without downtime.

## Preconditions

- You know which environments hold `APEX_MOLLIE_API_KEY`,
  `APEX_MOLLIE_API_BASE`, `APEX_MOLLIE_WEBHOOK_URL` and
  `APEX_MOLLIE_REDIRECT_URL`.
- You can deploy the api environment file and restart the service.

Mollie webhooks are unsigned, so there is no webhook signing key to rotate.
The API key is the only secret that authenticates the server-side payment
fetch and outbound payment creation.

## Procedure

1. In the Mollie dashboard, create a new API key with the least privilege the
   collection needs (payments and customers; payment links).
2. Write the new key into the deployment secret store, not into the repository.
3. Deploy to the test environment and confirm:
   - a webhook for a fetched payment returns `200`;
   - an unknown id returns `400 unknown_payment`;
   - a payment link opens and its `redirectUrl`/`webhookUrl` are the deployed
     URLs.
4. Flip the production secret during a quiet window and restart the api.
5. Revoke the old key after a short observation window.

## Validation

- The webhook logs show accepted payments and no fetch failures.
- A small monthly pass creates a payment link and records the collection.
- A recurring charge uses the stored `customerId`/`mandateId` with
  `sequenceType: "recurring"`.

## Notes

- A key belongs in the secret store, never in a commit; the pipeline scans for
  typos of this rule.
- Keep `APEX_MOLLIE_API_BASE` unchanged between rotations unless you are
  deliberately moving between Mollie's live and test API.
