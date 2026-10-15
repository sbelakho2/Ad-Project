# Adyen keys rotation

Rotate the Adyen credentials without downtime.

## Preconditions

- You know which environments hold `APEX_ADYEN_API_KEY`,
  `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_HMAC_KEY` and
  `APEX_ADYEN_CHECKOUT_BASE`.
- You can deploy the api environment file and restart the service.

## Procedure

1. In the Adyen dashboard, create a new restricted api key with the least
   privilege the collection needs.
2. Write the new key into the deployment secret store, not into the repository.
3. If you rotate the webhook signing key, reveal the new secret and store it as
   `APEX_ADYEN_HMAC_KEY`.
4. Deploy to the test environment and confirm a signed webhook returns `200`,
   a bad signature returns `400`, and the checkout link opens.
5. Flip the production secrets during a quiet window and restart the api.
6. Revoke the old keys after a short observation window.

## Validation

- The webhook logs show accepted items and no signature failures.
- A small monthly pass creates a payment link and records the collection.

## Notes

- A key belongs in the secret store, never in a commit; the pipeline scans for typos of this rule.
- Leave the checkout base unchanged between rotations.
