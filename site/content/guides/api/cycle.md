+++
title = "POST /v1/cycle/{publisher} | Apex API reference"
description = "Validated optional policy. Current shipped runner reports read_only."
template = "api.html"
weight = 6

[extra]
h1 = "POST /v1/cycle/{publisher}"
og_image = "og/guides.png"
+++

## Purpose

Validated optional policy. Current shipped runner reports read_only.

## Method and path

`POST /v1/cycle/{publisher}?platform=admob`. The `platform` query value is required and must be one of `applovin_max`, `levelplay` or `admob`.

## Authorization

The current router requires no authorization for this route. The shipped runner holds no platform credentials; the write path lives in a separately deployed service behind the same typed runner boundary.

## Request fields

The JSON body is optional. An empty body is the default request.

```json
{"policy": {"minLiftRateBp": 1800, "maxFeeMicros": 0}}
```

- `policy.minLiftRateBp` must lie in the business band 1500 to 2000 basis points.
- `policy.maxFeeMicros` must not be negative.
- Unknown fields are refused, so a typo cannot silently change what runs.
- A malformed body is 400 `{"error": "invalid_cycle_request"}`; a policy outside the band is 400 `{"error": "invalid_policy"}`.

## Sample response

```json
{
  "mode": "read_only",
  "publisher": "pub1",
  "records": 2,
  "critical_findings": 1,
  "note": "read-only deployment: the cycle audits the platform snapshot and reports the verified ledger state without mutating anything; applying write cycles requires platform credentials that this deployment does not hold"
}
```

## Failure states

- 400 `{"error": "platform_required"}`, `{"error": "unknown_platform"}`, `{"error": "invalid_publisher"}`, `{"error": "invalid_cycle_request"}`, `{"error": "invalid_policy"}` or `{"error": "bad_request"}` (duplicate query keys, malformed path).
- 404 when the platform snapshot is missing from the deployment.
- 422 `{"error": "ledger_unverified", "line": n}` when the publisher's ledger does not verify.
- 502 `{"error": "platform_unavailable"}` or 500 `{"error": "internal"}` for upstream and storage failures.
- 413 `{"error": "bad_request"}` for a body above 256 KiB.

## Idempotency

The shipped runner mutates nothing, so a repeated request repeats the same read. This route is not a public Apply button and not a full approval API.

## Current deployment

The route reports the current shipped runner: `read_only`.
