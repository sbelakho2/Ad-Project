# Creative Scanning API Reference

The Creative Scanning API provides brand safety verification for ad creatives before they're served.

## Base URL

```
https://api.adproject.io/v1/creative-scanning
```

## Authentication

All endpoints require API key authentication:

```
Authorization: Bearer YOUR_API_KEY
```

---

## Endpoints

### Scan Single Creative

Scans a single creative for brand safety issues.

**POST** `/scan`

#### Request Body

```json
{
  "creativeUrl": "https://cdn.example.com/ad/banner.png",
  "creativeType": "image",
  "advertiserDomain": "example.com",
  "categories": ["retail", "electronics"],
  "metadata": {
    "campaignId": "camp_123",
    "advertiserId": "adv_456"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `creativeUrl` | `string` | Yes | URL of the creative to scan |
| `creativeType` | `string` | Yes | Type: `image`, `video`, `html`, `vast` |
| `advertiserDomain` | `string` | No | Advertiser's domain |
| `categories` | `string[]` | No | IAB content categories |
| `metadata` | `object` | No | Additional metadata |

#### Response

```json
{
  "success": true,
  "result": {
    "creativeId": "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
    "status": "approved",
    "brandSafetyScore": 92,
    "categories": ["retail"],
    "flags": [],
    "malwareDetected": false,
    "redirectChain": [
      "https://cdn.example.com/ad/banner.png"
    ],
    "landingPageUrl": "https://example.com/landing",
    "scanDuration": 245,
    "scannedAt": "2025-01-09T12:00:00Z"
  },
  "cached": false
}
```

---

### Bulk Scan

Scans multiple creatives asynchronously.

**POST** `/scan/bulk`

#### Request Body

```json
{
  "creatives": [
    {
      "creativeUrl": "https://cdn.example.com/ad/banner1.png",
      "creativeType": "image"
    },
    {
      "creativeUrl": "https://cdn.example.com/ad/video1.mp4",
      "creativeType": "video"
    }
  ],
  "callbackUrl": "https://your-server.com/webhook/scan-complete"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `creatives` | `array` | Yes | Array of creatives (max 100) |
| `callbackUrl` | `string` | No | Webhook URL for completion |

#### Response

```json
{
  "success": true,
  "batchId": "batch_abc123",
  "statusUrl": "/api/creative-scanning/batch/batch_abc123"
}
```

---

### Get Batch Status

Get the status of a bulk scan batch.

**GET** `/batch/:batchId`

#### Response

```json
{
  "success": true,
  "batch": {
    "status": "complete",
    "total": 10,
    "completed": 10,
    "results": [
      {
        "creativeId": "...",
        "status": "approved",
        "brandSafetyScore": 95
      }
    ]
  }
}
```

| Batch Status | Description |
|--------------|-------------|
| `processing` | Scan in progress |
| `complete` | All creatives scanned |
| `failed` | Batch processing failed |

---

### Get Scan Result

Retrieve a previous scan result.

**GET** `/result/:creativeId`

#### Response

```json
{
  "success": true,
  "result": {
    "creativeId": "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
    "status": "approved",
    "brandSafetyScore": 92,
    "categories": ["retail"],
    "flags": [],
    "malwareDetected": false,
    "scannedAt": "2025-01-09T12:00:00Z"
  }
}
```

---

### Quick Validation

Fast approve/reject check without full scan details.

**POST** `/validate`

#### Request Body

```json
{
  "creativeUrl": "https://cdn.example.com/ad/banner.png",
  "creativeType": "image"
}
```

#### Response

```json
{
  "approved": true,
  "reason": null
}
```

Or if rejected:

```json
{
  "approved": false,
  "reason": "Malware detected in creative"
}
```

---

## Scan Result Object

| Field | Type | Description |
|-------|------|-------------|
| `creativeId` | `string` | Unique identifier (SHA256 hash of URL) |
| `status` | `string` | `approved`, `rejected`, `pending`, `error` |
| `brandSafetyScore` | `number` | Score from 0-100 |
| `categories` | `string[]` | Detected IAB categories |
| `flags` | `array` | Issues detected |
| `malwareDetected` | `boolean` | Whether malware was found |
| `redirectChain` | `string[]` | URLs in redirect chain |
| `landingPageUrl` | `string` | Final landing page URL |
| `scanDuration` | `number` | Scan time in milliseconds |
| `scannedAt` | `string` | ISO 8601 timestamp |

---

## Scan Flags

| Flag Type | Severity | Description |
|-----------|----------|-------------|
| `malware` | `critical` | Malware detected |
| `blocked_category` | `high` | Blocked content category |
| `suspicious_redirect` | `high` | Suspicious redirect chain |
| `invalid_url` | `high` | Invalid URL format |
| `url_shortener` | `medium` | URL shortener detected |
| `ip_address_url` | `medium` | Direct IP URL |
| `suspicious_javascript` | `high` | Suspicious JS patterns |
| `excessive_scripts` | `medium` | Too many external scripts |
| `excessive_iframes` | `medium` | Too many iframes |
| `invalid_vast` | `high` | Invalid VAST XML |
| `insecure_protocol` | `low` | Non-HTTPS URL |
| `large_file` | `low` | File exceeds size limit |

---

## Status Values

| Status | Description |
|--------|-------------|
| `approved` | Creative passed all checks |
| `rejected` | Creative failed critical checks |
| `pending` | Requires manual review |
| `error` | Scan encountered an error |

---

## Blocked Categories

The following IAB categories are automatically blocked:

- `adult`
- `gambling`
- `weapons`
- `drugs`
- `malware`
- `phishing`
- `hate_speech`
- `violence`
- `deceptive`

---

## Error Responses

```json
{
  "success": false,
  "error": "Invalid creative URL"
}
```

| HTTP Status | Description |
|-------------|-------------|
| `400` | Invalid request parameters |
| `401` | Missing or invalid API key |
| `404` | Creative or batch not found |
| `429` | Rate limit exceeded |
| `500` | Internal server error |

---

## Rate Limits

| Endpoint | Limit |
|----------|-------|
| `/scan` | 100 requests/minute |
| `/scan/bulk` | 10 requests/minute |
| `/validate` | 500 requests/minute |
| `/result/:id` | 1000 requests/minute |

---

## Webhooks

When using bulk scan with `callbackUrl`, you'll receive:

```json
{
  "batchId": "batch_abc123",
  "status": "complete",
  "results": [
    {
      "creativeId": "...",
      "status": "approved",
      "brandSafetyScore": 95
    }
  ]
}
```

---

## SDK Integration

### Pre-flight Check

```kotlin
// Android
suspend fun checkCreativeBeforeServing(creativeUrl: String): Boolean {
    val response = creativeScanning.validate(creativeUrl, "image")
    return response.approved
}
```

```swift
// iOS
func checkCreative(_ url: String) async -> Bool {
    let response = try await creativeScanning.validate(url: url, type: .image)
    return response.approved
}
```
