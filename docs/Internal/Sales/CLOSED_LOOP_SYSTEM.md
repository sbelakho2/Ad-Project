# 🔄 Closed-Loop Learning System

**Date:** January 2026  
**Status:** ✅ Deployed to Production (Test Server)  
**Migration:** `054_closed_loop_tracking.sql`  
**Execution Server:** Test Server (203.0.113.11:4001)

---

## Overview

The Closed-Loop Learning System enables the Thompson Sampler to automatically learn from email responses, creating a self-optimizing outbound sales engine.

> **⚠️ Architecture Note:** All closed-loop processing runs on the **Test Server** (sales-api:4001).
> The App Server control-plane is strictly a proxy client.

### Key Principle

> "Every email sent is an experiment. Every response (or lack thereof) is a data point."

Without closed-loop learning, the Thompson Sampler requires manual feedback input. With this system:
- ✅ Opens → Automatic success signal (+1 α)
- ✅ Clicks → Automatic success signal (+1 α)
- ✅ Replies → Automatic success signal (+1 α)
- ✅ Bounces → Automatic failure signal (+1 β)
- ✅ Unsubscribes → Automatic failure signal (+1 β)

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        EMAIL SEND FLOW                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  1. POST /compose                                                    │
│     ├── Thompson Sampler samples from arms                          │
│     ├── Picks subject line with highest sampled score               │
│     └── Returns { subject, body, subjectArmId }                     │
│                                                                      │
│  2. Send Email (via Amazon SES / Resend)                            │
│     ├── saveOutboundMessage() records:                               │
│     │   - contact_id                                                 │
│     │   - subject_arm_id  ← KEY FOR CLOSED LOOP                     │
│     │   - message_id (from ESP)                                      │
│     │   - sent_at                                                    │
│     └── INSERT INTO sa_outbound_messages                            │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘

                              ↓ (time passes)

┌─────────────────────────────────────────────────────────────────────┐
│                        WEBHOOK FLOW                                  │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  3. POST /webhook/email-events (or /webhook/sendgrid)               │
│     ├── Receives event: { messageId, event, email, timestamp }      │
│     ├── Note: Replies received via Self-Hosted SMTP on Port 2525    │
│     │   which POSTs to /webhook/sendgrid                            │
│     ├── Looks up outbound message by message_id or email            │
│     ├── Gets subject_arm_id from sa_outbound_messages               │
│     └── Persists to sa_email_events                                  │
│                                                                      │
│  4. Update Thompson Sampler                                          │
│     ├── Map event to outcome:                                        │
│     │   - open/click/reply → success = true                         │
│     │   - bounce/unsubscribe → success = false                      │
│     ├── recordOutcome(armId, success)                                │
│     │   - success=true → alpha++                                     │
│     │   - success=false → beta++                                     │
│     └── Mark outcome_recorded=true in sa_email_events               │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘

                              ↓ (next send)

┌─────────────────────────────────────────────────────────────────────┐
│                        LEARNING EFFECT                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  5. Next POST /compose                                               │
│     ├── Thompson Sampler now has updated α/β values                 │
│     ├── Arms with more opens get higher sampling probability        │
│     └── System naturally converges to best-performing subject       │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Database Tables

### sa_outbound_messages

Tracks every sent email with the Thompson Sampler arm used.

```sql
CREATE TABLE sa_outbound_messages (
  id UUID PRIMARY KEY,
  contact_id UUID NOT NULL REFERENCES sa_contacts(id),
  
  -- Content
  subject TEXT NOT NULL,
  body_text TEXT NOT NULL,
  body_html TEXT,
  
  -- CRITICAL: Links to Thompson Sampler arm
  subject_arm_id UUID REFERENCES sa_bandit_arms(id),
  template_id UUID,
  variation_hash VARCHAR(64),
  
  -- Status progression
  status VARCHAR(20) DEFAULT 'pending',
  sent_at TIMESTAMP,
  delivered_at TIMESTAMP,
  opened_at TIMESTAMP,
  clicked_at TIMESTAMP,
  replied_at TIMESTAMP,
  
  -- External tracking
  message_id VARCHAR(255),  -- From SendGrid/Mailgun
  provider VARCHAR(50),
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

### sa_email_events

Stores every webhook event for analytics and debugging.

```sql
CREATE TABLE sa_email_events (
  id UUID PRIMARY KEY,
  outbound_message_id UUID REFERENCES sa_outbound_messages(id),
  contact_id UUID REFERENCES sa_contacts(id),
  
  event_type VARCHAR(50) NOT NULL,  -- open, click, bounce, reply, unsubscribe
  event_data JSONB DEFAULT '{}',
  
  -- Bandit feedback
  arm_id UUID REFERENCES sa_bandit_arms(id),
  outcome_recorded BOOLEAN DEFAULT false,  -- Prevents double-counting
  
  source VARCHAR(50),  -- sendgrid, mailgun, webhook
  raw_payload JSONB,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

---

## Webhook Endpoints

### POST /api/autonomous/webhook/email-events

Generic webhook that works with any email provider.

**Request Format:**
```json
{
  "events": [
    {
      "messageId": "sendgrid-msg-123",
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
      "messageId": "sendgrid-msg-123",
      "event": "open",
      "armId": "uuid-of-arm",
      "success": true
    }
  ]
}
```

### POST /api/autonomous/webhook/sendgrid

Standard webhook for processing incoming emails.
> **Note:** Can be used with **Cloudflare Email Workers (Free)** by adapting the payload to match this format.

**Setup with Cloudflare (Zero Cost):**
1. Enable Cloudflare Email Routing for your domain.
2. Create an Email Worker that parses the incoming message (using `postal-mime`).
3. Have the worker POST a JSON object matching the SendGrid format below.

**Request Format (SendGrid-compatible):**
```json
{
  "from": "Sender Name <sender@example.com>",
  "to": "replies@yourdomain.com",
  "subject": "Re: Your email",
  "text": "The plain text body of the reply...",
  "html": "<p>The HTML body...</p>"
}
```

**Automatic Processing:**
1. Parses incoming email (from, subject, body)
2. Classifies using Naive Bayes (Clerk module)
3. Links to original outbound message via email address
4. If classification is `INTERESTED`, records positive bandit outcome

---

## Event-to-Outcome Mapping

| Email Event | Bandit Outcome | Thompson Sampler Update |
|-------------|----------------|-------------------------|
| `open` | **Success** | α += 1 |
| `click` | **Success** | α += 1 |
| `reply` | **Success** | α += 1 |
| `delivered` | (no update) | - |
| `bounce` | **Failure** | β += 1 |
| `unsubscribe` | **Failure** | β += 1 |
| `spam_report` | **Failure** | β += 1 |

---

## Code Flow

### 1. Sending an Email (Orchestrator)

```typescript
// autonomous-orchestrator.ts
async function sendEmail(contact: Contact, message: ComposedMessage) {
  // Send via ESP
  const sendResult = await sendgrid.send({
    to: contact.email,
    subject: message.subject,
    text: message.body,
  });
  
  // Save for closed-loop tracking
  await saveOutboundMessage({
    contactId: contact.id,
    subject: message.subject,
    bodyText: message.body,
    subjectArmId: message.subjectArmId,  // KEY!
    messageId: sendResult.messageId,
    provider: 'sendgrid',
    sentAt: new Date(),
  });
}
```

### 2. Processing Webhook (autonomous-api.ts)

```typescript
router.post('/webhook/email-events', async (req, res) => {
  const { events } = req.body;
  
  for (const event of events) {
    // Find outbound message
    const outbound = await findOutboundByMessageId(event.messageId);
    if (!outbound || !outbound.subject_arm_id) continue;
    
    // Persist event
    await queryOperator(`
      INSERT INTO sa_email_events 
        (outbound_message_id, contact_id, event_type, arm_id, source)
      VALUES ($1, $2, $3, $4, 'webhook')
    `, [outbound.id, outbound.contact_id, event.event, outbound.subject_arm_id]);
    
    // Update Thompson Sampler
    const isSuccess = ['open', 'click', 'reply'].includes(event.event);
    await recordOutcome(outbound.subject_arm_id, isSuccess);
  }
  
  res.json({ success: true });
});
```

---

## Monitoring & Debugging

### Check Bandit Convergence

```sql
SELECT 
  arm_name,
  total_trials,
  total_successes,
  ROUND(total_successes::decimal / NULLIF(total_trials, 0) * 100, 1) as success_rate,
  alpha,
  beta
FROM sa_bandit_arms
WHERE arm_type = 'subject_line' AND active = true
ORDER BY total_trials DESC;
```

### Check Recent Events

```sql
SELECT 
  ee.event_type,
  ee.created_at,
  om.subject,
  ba.arm_name,
  ee.outcome_recorded
FROM sa_email_events ee
JOIN sa_outbound_messages om ON ee.outbound_message_id = om.id
LEFT JOIN sa_bandit_arms ba ON ee.arm_id = ba.id
ORDER BY ee.created_at DESC
LIMIT 20;
```

### Check Event Distribution

```sql
SELECT 
  event_type,
  COUNT(*) as count,
  COUNT(CASE WHEN outcome_recorded THEN 1 END) as recorded
FROM sa_email_events
GROUP BY event_type
ORDER BY count DESC;
```

### Debug Missing Arm Links

```sql
-- Find outbound messages without arm tracking (shouldn't happen)
SELECT id, subject, sent_at
FROM sa_outbound_messages
WHERE subject_arm_id IS NULL
ORDER BY sent_at DESC
LIMIT 10;
```

---

## Best Practices

### 1. Ensure Arms Exist First

Before sending emails, make sure Thompson Sampler arms are seeded:

```sql
SELECT COUNT(*) FROM sa_bandit_arms WHERE arm_type = 'subject_line' AND active = true;
-- Should be > 0
```

### 2. Monitor Convergence

Arms should show increasing trial counts and diverging success rates:

```sql
SELECT arm_name, total_trials, 
  ROUND(total_successes::decimal / NULLIF(total_trials, 0) * 100, 1) as rate
FROM sa_bandit_arms
WHERE arm_type = 'subject_line'
ORDER BY rate DESC;
```

If all arms have similar rates after 100+ trials, consider:
- Adding more diverse subject lines
- Checking if webhooks are being received

### 3. Deduplicate Events

The `outcome_recorded` flag prevents double-counting. If a webhook fires twice for the same event, the second call should be a no-op.

---

## Related Documentation

- [Autonomous Sales API](../../Developer-Facing/AUTONOMOUS_SALES_API.md)
- [Autonomous Sales V2 Full Guide](AUTONOMOUS_SALES_V2.md)
- [Thompson Sampling Theory](https://en.wikipedia.org/wiki/Thompson_sampling)
