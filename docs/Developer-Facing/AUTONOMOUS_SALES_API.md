# Autonomous Sales API Reference

**App Server Proxy:** `https://admin.apexmediation.ee/api/autonomous` (proxies to test server)  
**Test Server Direct:** `http://203.0.113.11:4001/api/autonomous` (internal only)  
**Authentication:** API Key via `X-API-Key` header (or disabled in dev mode)  
**Last Updated:** January 2026 (v2.6 - Distributed Architecture)

---

## Architecture

> **⚠️ CRITICAL: All sales automation runs on the Test Server.**
> 
> The App Server control-plane is strictly a **proxy client**.
> Requests to `admin.apexmediation.ee/api/autonomous/*` are forwarded to the Test Server via SSH tunnel.

| Component | Server | Port | Role |
|-----------|--------|------|------|
| Control Plane | App (203.0.113.10) | 4000 | Proxy only |
| **Sales API** | **Test (203.0.113.11)** | 4001 | ALL orchestration |
| Edge Worker | Test (203.0.113.11) | 3500 | ML inference |

---

## Overview

The Autonomous Sales API provides programmatic access to the zero-cost outbound sales automation system. It enables:

- Lead discovery from app stores with **batch size protection**
- Heuristic lead scoring with **competitor detection ("Sniper" targeting)**
- Spintax message generation with Thompson Sampling optimization
- Email classification using Naive Bayes
- **Closed-loop feedback** via webhooks (auto-learns from email responses)
- **Competitor-filtered lead retrieval** for targeted outreach

### System Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_DISCOVER_BATCH` | 10 | Prevents HTTP timeouts during discovery |
| `MAX_SCORE_BATCH` | 100 | Prevents HTTP timeouts during scoring |

---

## Endpoints

### Health & Status

#### GET /health
Health check endpoint.

**Response:**
```json
{
  "status": "healthy",
  "service": "autonomous-sales",
  "timestamp": "2026-01-25T00:39:06.515Z"
}
```

#### GET /stats
System-wide statistics.

**Response:**
```json
{
  "success": true,
  "stats": {
    "discovery": {
      "sources": 5,
      "lastScrape": "2026-01-24T15:30:00.000Z"
    },
    "scoring": {
      "contacts": 250,
      "scored": 245,
      "avgScore": 52.3
    },
    "optimizer": {
      "arms": 8,
      "trials": 1200,
      "convergence": 75
    },
    "inbox": {
      "total": 45,
      "pendingReview": 3
    }
  }
}
```

---

### Lead Sources

#### GET /sources
List all lead discovery sources.

**Response:**
```json
{
  "success": true,
  "sources": [
    {
      "id": "uuid",
      "name": "US Games",
      "sourceType": "itunes",
      "searchQuery": "arcade game",
      "category": "Games",
      "region": "us",
      "status": "active",
      "lastScrapedAt": "2026-01-24T15:30:00.000Z",
      "totalContactsFound": 150
    }
  ]
}
```

#### POST /sources
Create a new lead source.

**Request:**
```json
{
  "name": "US Games",
  "sourceType": "itunes",
  "searchQuery": "arcade game",
  "category": "Games",
  "region": "us"
}
```

**Response:**
```json
{
  "success": true,
  "sourceId": "uuid"
}
```

---

### Discovery

#### POST /discover
Run discovery job for a source.

**Request:**
```json
{
  "sourceId": "uuid",
  "batchSize": 5
}
```

> **Note:** `batchSize` is capped at `MAX_DISCOVER_BATCH` (10) to prevent HTTP timeouts.
> If you request more, it will be automatically reduced and logged.

**Response:**
```json
{
  "success": true,
  "discovered": 5,
  "source": {
    "id": "uuid",
    "name": "US Games",
    "totalContactsFound": 200
  }
}
```

---

### Scoring

#### POST /score
Score contacts using heuristic analysis.

**Request (single contact):**
```json
{
  "contactId": "uuid"
}
```

**Request (batch):**
```json
{
  "limit": 100
}
```

> **Note:** `limit` is capped at `MAX_SCORE_BATCH` (100) to prevent HTTP timeouts.

**Response:**
```json
{
  "success": true,
  "scored": 100,
  "byTier": {
    "hot": 5,
    "warm": 25,
    "cold": 55,
    "ice": 15
  }
}
```

**Tier Classification:**

| Tier | Score Range | Description |
|------|-------------|-------------|
| `hot` | 70+ | High-value, uses competitor networks |
| `warm` | 50-69 | Good potential, some positive signals |
| `cold` | 30-49 | Low signals, worth nurturing |
| `ice` | <30 | Minimal signals, low priority |

---

### Templates

#### GET /templates
List spintax templates.

**Query Parameters:**
- `type` (optional): Filter by template type (`email`, `followup`, `subject`)

**Response:**
```json
{
  "success": true,
  "templates": [
    {
      "id": "uuid",
      "name": "Initial Outreach - Ad Mediation",
      "description": "First touch template for app developers",
      "templateType": "email",
      "subjectSpintax": "{Quick question about|Question re:} {{app_name}}",
      "bodySpintax": "{Hi|Hello} {{first_name}}...",
      "availableVariables": ["first_name", "app_name", "app_genre"],
      "timesUsed": 150,
      "totalOpens": 45,
      "totalReplies": 12,
      "openRate": 30.0,
      "replyRate": 8.0,
      "active": true
    }
  ]
}
```

#### POST /templates
Create a new template.

**Request:**
```json
{
  "name": "Game Developer Special",
  "description": "Optimized for mobile game developers",
  "templateType": "email",
  "subjectSpintax": "{Boosting|Maximizing} {{app_name}} ad revenue",
  "bodySpintax": "Hey {{first_name}},\n\n{Love|Impressed by} {{app_name}}...",
  "availableVariables": ["first_name", "app_name", "app_genre"]
}
```

**Response:**
```json
{
  "success": true,
  "templateId": "uuid"
}
```

#### POST /templates/preview
Generate preview variations of a template.

**Request:**
```json
{
  "subjectSpintax": "{Quick question|Question} about {{app_name}}",
  "bodySpintax": "{Hi|Hello} {{first_name}}...",
  "context": {
    "firstName": "John",
    "appName": "PuzzleQuest"
  },
  "count": 5
}
```

**Response:**
```json
{
  "success": true,
  "variations": [
    {
      "subject": "Quick question about PuzzleQuest",
      "body": "Hi John..."
    },
    {
      "subject": "Question about PuzzleQuest",
      "body": "Hello John..."
    }
  ]
}
```

---

### Bandit Arms (A/B Testing)

#### GET /arms
List Thompson Sampler arms.

**Query Parameters:**
- `type` (optional): Filter by arm type (`subject_line`, `template`, `send_time`)

**Response:**
```json
{
  "success": true,
  "arms": [
    {
      "id": "uuid",
      "armType": "subject_line",
      "armName": "Direct Question",
      "armValue": "Quick question about {{app_name}} monetization",
      "alpha": 35,
      "beta": 16,
      "totalTrials": 50,
      "totalSuccesses": 34,
      "empiricalRate": 0.68,
      "active": true
    }
  ],
  "stats": {
    "totalArms": 4,
    "activeArms": 4,
    "totalTrials": 200,
    "bestPerformer": {
      "name": "Direct Question",
      "rate": 0.68
    },
    "convergenceScore": 75
  }
}
```

#### POST /arms
Create a new arm.

**Request:**
```json
{
  "armType": "subject_line",
  "armName": "Revenue Focus",
  "armValue": "{{app_name}} - leaving ad revenue on the table?"
}
```

**Response:**
```json
{
  "success": true,
  "armId": "uuid"
}
```

---

### Message Composition

#### POST /compose
Generate a personalized message using Thompson Sampling.

**Request:**
```json
{
  "contactId": "uuid",
  "firstName": "John",
  "company": "GameCo",
  "appName": "PuzzleQuest",
  "appGenre": "Games"
}
```

**Response:**
```json
{
  "success": true,
  "message": {
    "subject": "Quick question about PuzzleQuest monetization",
    "body": "Hi John,\n\nI noticed PuzzleQuest on the App Store...",
    "templateId": "uuid",
    "subjectArmId": "uuid",
    "variationHash": "abc123def456"
  }
}
```

---

### Email Classification

#### GET /inbox
List classified emails.

**Query Parameters:**
- `status` (optional): Filter by status (`pending_review`, `reviewed`, `all`)
- `limit` (optional): Number of results (default: 50)

**Response:**
```json
{
  "success": true,
  "messages": [
    {
      "id": "uuid",
      "fromEmail": "john@example.com",
      "subject": "Re: PuzzleQuest monetization",
      "bodyText": "Thanks for reaching out...",
      "classification": "INTERESTED",
      "classificationConfidence": 0.87,
      "classificationMethod": "naive_bayes",
      "requiresHumanReview": false,
      "receivedAt": "2026-01-24T10:30:00.000Z"
    }
  ],
  "stats": {
    "totalMessages": 45,
    "byCategory": {
      "INTERESTED": 12,
      "NOT_INTERESTED": 20,
      "UNSUBSCRIBE": 5,
      "OUT_OF_OFFICE": 3,
      "BOUNCE": 2,
      "UNKNOWN": 3
    },
    "pendingReview": 3,
    "averageConfidence": 0.82
  }
}
```

#### POST /classify
Classify an email.

**Request:**
```json
{
  "subject": "Re: Your email about our app",
  "body": "Thanks for reaching out! This sounds interesting...",
  "fromEmail": "john@example.com"
}
```

**Response:**
```json
{
  "success": true,
  "classification": {
    "category": "INTERESTED",
    "confidence": 0.87,
    "method": "naive_bayes",
    "requiresReview": false,
    "messageId": "uuid"
  }
}
```

#### POST /inbox/review
Submit human review for a classified email.

**Request:**
```json
{
  "messageId": "uuid",
  "correctCategory": "INTERESTED"
}
```

**Response:**
```json
{
  "success": true
}
```

---

### Feedback

#### POST /feedback
Record email event for Thompson Sampler learning.

**Request:**
```json
{
  "armId": "uuid",
  "eventType": "open"
}
```

Event types: `open`, `reply`, `bounce`

**Response:**
```json
{
  "success": true
}
```

---

### Competitor Leads (Sniper Targeting) 🎯

#### GET /competitor-leads
Get leads using specific competitor networks. **This is the highest-value endpoint** for targeted outreach.

**Query Parameters:**
- `competitor` (optional): Filter by competitor name (partial match)
- `tier` (optional): Filter by competitor tier (1, 2, or 3)
- `minScore` (optional): Minimum overall score (default: 30)
- `limit` (optional): Max results (default: 50)

**Competitor Tiers:**

| Tier | Networks | Boost | Description |
|------|----------|-------|-------------|
| 1 | IronSource, Unity Ads, AppLovin MAX | +45-50 | Direct competitors, highest priority |
| 2 | Mintegral, Vungle, Chartboost, Fyber, InMobi | +30-35 | Major players, good opportunities |
| 3 | AdColony, Tapjoy, StartApp, Ogury | +20-25 | Legacy/weaker networks |

**Response:**
```json
{
  "success": true,
  "leads": [
    {
      "id": "uuid",
      "email": "dev@gamecompany.com",
      "first_name": "John",
      "last_name": "Doe",
      "company_name": "Game Company",
      "app_name": "PuzzleQuest",
      "app_genre": "Games",
      "score": 85,
      "status": "new",
      "top_competitor": "IronSource",
      "competitor_tier": 1,
      "competitor_domain": "ironsrc.com"
    }
  ],
  "count": 15,
  "byCompetitor": {
    "IronSource": [...],
    "Unity Ads": [...]
  },
  "competitorStats": [
    { "competitor": "IronSource", "count": 8, "avgScore": 82 },
    { "competitor": "Unity Ads", "count": 7, "avgScore": 78 }
  ]
}
```

**Example Queries:**
```bash
# All Tier 1 competitor leads
GET /competitor-leads?tier=1

# IronSource users with score > 50
GET /competitor-leads?competitor=ironsource&minScore=50

# Top 20 Unity Ads users
GET /competitor-leads?competitor=unity&limit=20
```

---

### Webhooks (Closed-Loop Learning) 🔄

The closed-loop system automatically learns from email responses to optimize subject lines.

#### POST /webhook/email-events
Generic email events webhook. Updates Thompson Sampler based on email outcomes.

**Supported Events:**

| Event | Bandit Outcome | Description |
|-------|----------------|-------------|
| `open` | Success (+1 α) | Email was opened |
| `click` | Success (+1 α) | Link was clicked |
| `reply` | Success (+1 α) | User replied |
| `bounce` | Failure (+1 β) | Email bounced |
| `unsubscribe` | Failure (+1 β) | User unsubscribed |

**Request:**
```json
{
  "events": [
    {
      "messageId": "abc123",
      "event": "open",
      "timestamp": "2026-01-25T10:30:00Z",
      "email": "john@example.com"
    }
  ]
}
```

**Response:**
```json
{
  "success": true,
  "processed": 1,
  "outcomes": [
    {
      "messageId": "abc123",
      "event": "open",
      "armId": "uuid",
      "success": true
    }
  ]
}
```

#### POST /webhook/sendgrid
SendGrid Inbound Parse webhook. Processes incoming email replies.

**Setup:** Configure SendGrid MX record to point to:
```
https://admin.apexmediation.ee/api/autonomous/webhook/sendgrid
```

**Automatic Actions:**
1. Classifies incoming email using Naive Bayes
2. Links response to original outbound message
3. Updates Thompson Sampler arm (positive outcome if `INTERESTED`)
4. Queues for human review if low confidence

**Request (SendGrid format):**
```json
{
  "from": "John Doe <john@example.com>",
  "to": "sales@apexmediation.ee",
  "subject": "Re: Quick question about PuzzleQuest",
  "text": "Sounds interesting! Let's chat...",
  "html": "<p>Sounds interesting! Let's chat...</p>"
}
```

**Response:**
```json
{
  "success": true,
  "classification": "INTERESTED",
  "messageId": "uuid",
  "banditUpdated": true
}
```

---

## Error Responses

All endpoints return errors in this format:

```json
{
  "error": "Error message describing what went wrong"
}
```

Common HTTP status codes:
- `400` - Bad request (missing required fields)
- `404` - Resource not found
- `500` - Internal server error

---

## Rate Limits

No rate limits are currently enforced for internal use.
Batch size limits (MAX_DISCOVER_BATCH=10, MAX_SCORE_BATCH=100) prevent timeout issues.

---

## Database Tables

The system uses these PostgreSQL tables:

### Core Tables
| Table | Purpose |
|-------|---------|
| `sa_sources` | Lead discovery sources (iTunes, etc.) |
| `sa_contacts` | Discovered contacts with scoring |
| `sa_heuristic_scores` | Detailed scoring breakdowns |
| `sa_scrape_jobs` | Discovery job history |

### Messaging Tables
| Table | Purpose |
|-------|---------|
| `sa_spintax_templates` | Email templates with spintax |
| `sa_bandit_arms` | Thompson Sampler arms |
| `sa_outbound_messages` | Sent emails with arm tracking |

### Inbox Tables
| Table | Purpose |
|-------|---------|
| `sa_inbox_messages` | Received email classifications |
| `sa_email_events` | Webhook events (opens, clicks, etc.) |
| `sa_bayes_training` | Naive Bayes training data |

### Competitor Tables
| Table | Purpose |
|-------|---------|
| `sa_competitor_detections` | Per-contact competitor tracking |

---

## Related Documentation

- [Autonomous Sales V2 Full Guide](../Internal/Sales/AUTONOMOUS_SALES_V2.md)
- [Autonomous Sales V2 Quick Reference](../Internal/Sales/AUTONOMOUS_SALES_V2_QUICK_REF.md)
- [Closed Loop System](../Internal/Sales/CLOSED_LOOP_SYSTEM.md)
