# Autonomous Sales API Reference

**Base URL:** `https://admin.apexmediation.ee/api/autonomous`  
**Authentication:** Internal use only (Control Plane)

---

## Overview

The Autonomous Sales API provides programmatic access to the zero-cost outbound sales automation system. It enables:

- Lead discovery from app stores
- Heuristic lead scoring
- Spintax message generation with Thompson Sampling optimization
- Email classification using Naive Bayes

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
  "sourceId": "uuid"
}
```

**Response:**
```json
{
  "success": true,
  "discovered": 50,
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

**Response:**
```json
{
  "success": true,
  "scored": 100,
  "byTier": {
    "whale": 12,
    "qualified": 45,
    "cold": 43
  }
}
```

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

---

## Related Documentation

- [Autonomous Sales V2 Full Guide](../Internal/Sales/AUTONOMOUS_SALES_V2.md)
- [Autonomous Sales V2 Quick Reference](../Internal/Sales/AUTONOMOUS_SALES_V2_QUICK_REF.md)
