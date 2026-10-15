+++
title = "GET /v1/audit/{platform} | Apex API reference"
description = "Audit from configured snapshot; unknown platform 400, missing snapshot 404."
template = "api.html"
weight = 3

[extra]
h1 = "GET /v1/audit/{platform}"
og_image = "og/guides.png"
+++

## Purpose

Audit from configured snapshot; unknown platform 400, missing snapshot 404.

## Method and path

`GET /v1/audit/{platform}` where `{platform}` is one of `applovin_max`, `levelplay` or `admob`.

## Authorization

None. The endpoint reads a snapshot file configured in the deployment.

## Request fields

None. The platform path value selects the snapshot.

## Sample response

```json
{
  "snapshotDigest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "findings": [
    {
      "rule": "stale_floor",
      "severity": "warning",
      "platform": "admob",
      "subject": "ad_unit:u1:country:EE",
      "evidence": {
        "ad_unit_id": "u1",
        "country": "EE",
        "floor_ecpm_micros": 4000000,
        "revenue_30d_micros": 0,
        "impressions_30d": 50000,
        "ad_unit_total_revenue_30d_micros": 450000000,
        "ad_unit_total_impressions_30d": 500000,
        "ad_unit_ecpm_micros": 900000,
        "ecpm_unavailable_reason": null,
        "recovery_model": "floor_blocks_fill",
        "estimated_monthly_recovery_micros": 45000000
      },
      "message": "stale floor on ad unit `u1` in country `EE`: floor 4000000 micros set, pair revenue 0 micros over 50000 impressions; ad unit eCPM 900000 micros implies 45000000 micros/month recoverable (model: floor_blocks_fill)",
      "estimatedMonthlyRecovery": 45000000
    }
  ],
  "totalEstimatedMonthlyRecovery": 45000000,
  "generatedBy": "apex-audit v0.1.0"
}
```

Money is integer micros in the evidence; `snapshotDigest` is the SHA-256 digest of the audited snapshot. Findings sort by severity, then rule, then subject.

## Failure states

- 400 `{"error": "unknown_platform"}` for a platform outside the three supported values; 400 `{"error": "bad_request"}` for a malformed or non-UTF-8 path segment.
- 404 `{"error": "not_found"}` when no snapshot file exists for the platform in this deployment.
- 500 `{"error": "internal"}` when the snapshot file cannot be parsed or declares a different platform.

## Idempotency

A safe read of the configured snapshot; repeating it reads the same file.

## Current deployment

This route audits the configured snapshot for the platform. For a live read-only audit over a publisher's own keys, use `POST /v1/audit/{platform}`.
