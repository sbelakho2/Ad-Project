+++
title = "POST /v1/mollie/webhook | Apex API reference"
description = "Accept a Mollie payment id, fetch the payment server-side, and record it."
template = "api.html"
weight = 8

[extra]
h1 = "POST /v1/mollie/webhook"
og_image = "og/guides.png"
+++

## Purpose

Accept a Mollie payment notification and record the fetched payment. Mollie
webhooks are **not signed**: the delivery says only which payment changed.

## Method and path

`POST /v1/mollie/webhook`

## Authorization

No bearer token. The endpoint is authenticated by fetching the named payment
from Mollie with the deployment's API key (`Authorization: Bearer`). The POST
body is never trusted for status, amount or metadata. Without a configured
Mollie client the route answers 503
`{"error": "mollie_webhook_not_configured"}`.

## Request fields

Mollie sends `application/x-www-form-urlencoded`, with one `id` field:

```text
id=tr_5Kp3MnOpQr
```

Only the id is read. A body that also claims a status, amount or metadata has
those extra fields ignored.

## Sample response

```json
{"received": true, "id": "tr_5Kp3MnOpQr", "status": "paid", "new": true}
```

A replay of the same fetched `(payment id, status)` answers the same shape with
`"new": false` and records nothing twice.

## Validation

After the fetch, the route validates:

- the fetched payment id equals the posted id;
- the fetched status is a known Mollie status;
- `metadata.reference` is present and names an invoice or watch reference this
  deployment issued;
- the fetched amount and currency match the expected invoice or tier amount
  (`metadata.amount` must also agree with the fetched amount);
- for a watch payment, the amount equals the tier price the subscriber was
  quoted.

A paid watch payment activates the subscriber once and records the fetched
`customerId`/`mandateId` for later recurring charges.

## Failure states

- 400 `{"error": "malformed_event"}` for a missing, duplicate or non-Mollie id,
  or a body that is not form-encoded text.
- 400 `{"error": "unknown_payment"}` when Mollie does not return the named
  payment (a forged or unknown id).
- 400 `{"error": "invalid_payment"}` for a status/amount/reference/currency
  that does not validate, including underpayment.
- 502 `{"error": "platform_unavailable"}` when the Mollie fetch itself fails.
- 503 `{"error": "mollie_webhook_not_configured"}` without a configured Mollie
  client and transport.
- 503 `{"error": "collection_store_not_configured"}` without a collection store.

## Idempotency

Replay-protected on the fetched `(payment id, status)` pair. Mollie's retries
are acknowledged with a 2xx, but a repeated paid delivery is not applied twice.

## Current deployment

Accepted payments are recorded in the deployment's append-only collection log.
A paid invoice event appends the publisher's `paid` outcome; a paid watch
payment activates the setup watch. The durable payment projection is separate
from this endpoint.
