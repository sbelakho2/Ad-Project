+++
title = "POST /v1/audit/{platform} | Apex API reference"
description = "Live read-only audit using publisher body and deployment bearer authorization. Disabled deployment 503."
template = "api.html"
weight = 4

[extra]
h1 = "POST /v1/audit/{platform}"
og_image = "og/guides.png"
+++

## Purpose

Live read-only audit using publisher body and deployment bearer authorization. Disabled deployment 503.

## Method and path

`POST /v1/audit/{platform}` where `{platform}` is one of `applovin_max`, `levelplay` or `admob`.

## Authorization

`Authorization: Bearer <token>`, where the token is the deployment's live-audit token. Without a configured live auditor the route answers 503 `{"error": "live_audit_not_configured"}`.

## Request fields

```json
{"publisher": "northstar"}
```

`publisher` must match `^[A-Za-z0-9_-]{1,64}$`. The audit runs on the publisher's own platform keys from the deployment's encrypted credential store. The platform is never written to.

## Sample response

```json
{
  "snapshotDigest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "findings": [],
  "totalEstimatedMonthlyRecovery": 0,
  "generatedBy": "apex-audit v0.1.0"
}
```

The response is the same `AuditReport` shape as `GET /v1/audit/{platform}`.

## Failure states

- 400 `{"error": "invalid_publisher"}` or `{"error": "unknown_platform"}` or `{"error": "bad_request"}`.
- 401 `{"error": "unauthorized"}` for a missing or wrong bearer token.
- 502 `{"error": "platform_unavailable"}` when the platform call fails; the upstream detail stays in the server log.
- 503 `{"error": "live_audit_not_configured"}` when the deployment has no live auditor.

## Idempotency

Read-only against the platform; repeating the call repeats the read. The live snapshot is persisted for the read-only audit source.

## Current deployment

The current boundary is one deployment-wide bearer token. A tenant-safe HTML access path must not expose that token; it stays in the `Authorization` header and never appears in HTML, URLs or logs.
