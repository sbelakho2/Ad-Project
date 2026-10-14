# Developer-Facing API Documentation

> **Last Updated:** January 30, 2026

This section contains API references and integration guides for developers building with the ApexMediation platform.

---

## 📚 API References

### Core APIs

| API | Description | Documentation |
|-----|-------------|---------------|
| **SSAI API** | Server-Side Ad Insertion endpoints | [SSAI API Reference](#ssai-api) |
| **RTB API** | Real-time bidding endpoints | the billing API reference (archived with the mediation platform) |
| **Billing API** | Billing and subscription management | [Backend Billing](../Internal/Backend/BILLING_API.md) |
| **Sales Automation API** | Outbound sales automation | [Autonomous Sales API](./AUTONOMOUS_SALES_API.md) |

### SDK Documentation

| Platform | Location | Language |
|----------|----------|----------|
| Web SDK | `/sdk/web/` | TypeScript |
| iOS SDK | `/sdk/core/ios/` | Swift |
| Android SDK | `/sdk/core/android/` | Kotlin |
| tvOS SDK | `/sdk/ctv/tvos/` | Swift |
| Android TV SDK | `/sdk/ctv/android-tv/` | Kotlin |

See [SDK Integration Guide](../Customer-Facing/00_OVERVIEW.md) for complete documentation.

---

## 🎬 SSAI API

Server-Side Ad Insertion API for video ad serving.

### Base URL

```
Production: https://api.apexmediation.com/api/v1/ssai
Health:     https://api.apexmediation.com/api/v1/ssai/health
```

### Authentication

All SSAI endpoints require a valid API key:

```http
Authorization: Bearer <api_key>
```

### Endpoints

#### Health Check

```http
GET /api/v1/ssai/health
```

**Response:**
```json
{
  "healthy": true,
  "components": {
    "redis": { "status": "healthy", "latencyMs": 11 },
    "database": { "status": "healthy", "latencyMs": 21 },
    "partners": { "status": "healthy", "latencyMs": 50 }
  }
}
```

#### Create Session

```http
POST /api/v1/ssai/session
Content-Type: application/json

{
  "contentId": "video-123",
  "contentDuration": 600,
  "adBreakConfig": {
    "preroll": true,
    "midrollInterval": 300,
    "postroll": true
  },
  "deviceInfo": {
    "platform": "web",
    "playerVersion": "1.0.0"
  }
}
```

**Response:**
```json
{
  "sessionId": "sess_abc123",
  "manifestUrl": "https://cdn.example.com/manifest/sess_abc123.m3u8",
  "trackingUrl": "https://api.apexmediation.com/api/v1/ssai/track/sess_abc123",
  "expiresAt": "2026-01-30T12:00:00Z"
}
```

#### Get Manifest

```http
GET /api/v1/ssai/manifest/{sessionId}
```

Returns HLS or DASH manifest with stitched ad breaks.

#### Fire Tracking Event

```http
POST /api/v1/ssai/track/{sessionId}
Content-Type: application/json

{
  "event": "impression",
  "adId": "ad_001",
  "podId": "pod_001",
  "slotId": "slot_001",
  "timestamp": "2026-01-30T10:00:00Z"
}
```

### VAST/VMAP Parsing (Internal API)

For internal SSAI pipeline use. See [VAST/VMAP Parser Documentation](../SSAI/VAST_VMAP_PARSER.md).

```typescript
import { parseVAST, parseVMAP, AdRequestHandler } from '@services/ssai/vast';

// Parse VAST
const vastResult = await parseVAST(vastXml);
if (vastResult.success) {
  console.log(`Parsed ${vastResult.document.ads.length} ads`);
}

// Parse VMAP
const vmapResult = await parseVMAP(vmapXml);
if (vmapResult.success) {
  console.log(`Parsed ${vmapResult.document.adBreaks.length} breaks`);
}

// Full request handling
const handler = new AdRequestHandler({
  contentDuration: 600,
  resolveWrappers: true,
});
const result = await handler.processAdResponse(xml);
```

---

## 💳 Billing & Tiers

See [Billing and Tiers](./Billing-and-Tiers.md) for:

- Platform fee structure (0% - 2.5%)
- Tier features (Starter, Growth, Scale, Enterprise)
- Access controls per tier
- Billing integration

---

## 🤖 Sales Automation API

See [Autonomous Sales API](./AUTONOMOUS_SALES_API.md) for:

- Prospect management endpoints
- Campaign automation
- Thompson Sampling configuration
- Webhook integration

---

## 📖 Related Documentation

- [SSAI Improvements](../SSAI/SSAI_IMPROVEMENTS.md) - Full SSAI specification
- [VAST/VMAP Parser](../SSAI/VAST_VMAP_PARSER.md) - Parser documentation
- the platform README (archived) - Backend setup guide
- the billing API reference (archived with the mediation platform) - Full API specification
