+++
title = "GET /v1/ledger/{publisher} | Apex API reference"
description = "Publisher bearer token; verified records only. 401, 422 and 503 are meaningful."
template = "api.html"
weight = 5

[extra]
h1 = "GET /v1/ledger/{publisher}"
og_image = "og/guides.png"
+++

## Purpose

Publisher bearer token; verified records only. 401, 422 and 503 are meaningful.

## Method and path

`GET /v1/ledger/{publisher}`

## Authorization

`Authorization: Bearer <token>` with the publisher's ledger token from the deployment's configured token set. A publisher with no configured token fails closed with 503. Tokens stay out of query strings.

## Request fields

The publisher path value must match `^[A-Za-z0-9_-]{1,64}$`.

## Sample response

```json
{
  "publisher": "pub1",
  "mode": "read_only",
  "records": [
    {
      "type": "changeApplied",
      "seq": 0,
      "atUnixMs": 1700000001000,
      "platform": "admob",
      "adUnitId": "u1",
      "change": {"type": "countryFloorSet", "country": "EE", "floorEcpm": 2500000},
      "idempotencyKey": "admob-floor:u1:EE",
      "prevValue": "0",
      "prevHash": "0000000000000000000000000000000000000000000000000000000000000000",
      "hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    }
  ]
}
```

Each record carries the chain envelope (`seq`, `atUnixMs`, `platform`, `prevHash`, `hash`) plus its variant fields. Timestamps are Unix milliseconds.

## Failure states

- 400 `{"error": "invalid_publisher"}` or `{"error": "bad_request"}`.
- 401 `{"error": "unauthorized"}` for a missing or wrong token.
- 404 `{"error": "not_found"}` when no ledger file exists; a symlink escaping the configured root is refused the same way.
- 422 `{"error": "ledger_unverified", "line": n}` when verification fails; nothing is served from an unverified ledger.
- 503 `{"error": "ledger_auth_not_configured"}` when the publisher has no configured token.

## Idempotency

A safe read; repeat reads return the same verified records while the ledger is unchanged.

## Current deployment

Fail-closed verification: the ledger is read once through the verifier, and records are only served from bytes that passed verification.
