+++
title = "POST /v1/leads | Apex API reference"
description = "Valid JSON or URL-encoded lead. Success is 202 received."
template = "api.html"
weight = 7

[extra]
h1 = "POST /v1/leads"
og_image = "og/guides.png"
+++

## Purpose

Valid JSON or URL-encoded lead. Success is 202 received.

## Method and path

`POST /v1/leads`

## Authorization

None. The deployment must configure a lead store; without one the route answers 503 `{"error": "lead_capture_not_configured"}`.

## Request fields

JSON body:

```json
{
  "publisher": "northstar",
  "email": "founder@studio.example",
  "platforms": ["applovin_max"],
  "monthlyRevenueRange": "10k-50k",
  "notes": "One app, two ad units."
}
```

- `publisher` is the machine-safe name (`^[A-Za-z0-9_-]{1,64}$`). A human display name with spaces travels through the separate HTML form.
- `platforms` accepts `applovin_max`, `levelplay` and `admob`; at least one is required.
- `monthlyRevenueRange` and `notes` are optional.

The same fields are accepted as `application/x-www-form-urlencoded`, where a repeated `platforms` key joins its values with a comma.

## Sample response

```json
{"received": true}
```

with status 202.

## Failure states

- 400 `{"error": "invalid_lead"}` for a bad email, an invalid publisher name, or unknown or empty platforms.
- 413 `{"error": "bad_request"}` for a body above 256 KiB.
- 503 `{"error": "lead_capture_not_configured"}` without a configured lead store.

## Idempotency

Not idempotent: every accepted request is appended to the deployment's lead log.

## Current deployment

The JSON contract is as documented. The HTML form handler behind `/free-audit/` is separate and answers a 303 redirect to a confirmation page.
