+++
title = "POST /v1/adyen/webhook | Apex API reference"
description = "Verify provider notification signatures and parse each item."
template = "api.html"
weight = 8

[extra]
h1 = "POST /v1/adyen/webhook"
og_image = "og/guides.png"
+++

## Purpose

Verify provider notification signatures and parse each item.

## Method and path

`POST /v1/adyen/webhook`

## Authorization

No bearer token. Each notification item carries `additionalData.hmacSignature`, verified against the deployment's HMAC key over the signed fields. Without a configured key the route answers 503 `{"error": "adyen_webhook_not_configured"}`.

## Request fields

The Adyen notification envelope:

```json
{
  "live": "false",
  "notificationItems": [
    {
      "NotificationRequestItem": {
        "pspReference": "9916934567890123",
        "originalReference": "",
        "merchantAccountCode": "ApexMerchant",
        "merchantReference": "apex-fee:pub1:exp:2026-10",
        "amount": {"value": 1000, "currency": "USD"},
        "eventCode": "AUTHORISATION",
        "success": "true",
        "additionalData": {"hmacSignature": "DCVYHJK+/X/7a7OmfIVJTaIZylmJtM+z6OT1b5Ky/D0="}
      }
    }
  ]
}
```

The signature above is a pinned test vector, not a credential.

## Sample response

```json
{"received": true, "items": 1, "new": 1}
```

A replay of an already-recorded item answers the same shape with `"new": 0`.

## Failure states

- 400 `{"error": "invalid_signature"}` when a digest does not match; the response never says which check failed.
- 400 `{"error": "malformed_event"}` for a missing or unparseable signature, a malformed envelope, or an empty item list.
- 503 `{"error": "adyen_webhook_not_configured"}` without a configured HMAC key.
- 503 `{"error": "collection_store_not_configured"}` without a configured collection store.

## Idempotency

Replay-protected on `eventCode` plus `pspReference`: a repeated delivery is accepted, but `new` counts only the events that request recorded.

## Current deployment

Accepted events are recorded in the deployment's append-only collection log. The durable payment projection is separate from this endpoint.
