# 🤖 Autonomous Sales V2 - Quick Reference

**Version:** v2.6 (Distributed Architecture - January 2026)

## Architecture

> **⚠️ App Server is PROXY ONLY. All sales automation runs on Test Server.**

| Server | IP | Role | Key Service |
|--------|-----|------|-------------|
| App Server | 46.62.252.96 | Proxy | control-plane (4000) → forwards to test server |
| **Test Server** | 157.90.126.205 | **Execution** | sales-api (4001), edge-worker (3500) |
| DB Server | 77.42.44.35 | Database | PostgreSQL (via App Server tunnel only) |

## API Endpoints

**Via App Server proxy:** `https://admin.apexmediation.ee/api/autonomous/`  
**Direct to Test Server:** `http://157.90.126.205:4001/api/autonomous/` (internal only)

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Health check |
| `/stats` | GET | System dashboard |
| `/sources` | GET/POST | Lead source management |
| `/discover` | POST | Run discovery job (max 10 batch) |
| `/score` | POST | Score contacts (max 100 batch) |
| `/templates` | GET/POST | Spintax templates |
| `/templates/preview` | POST | Preview variations |
| `/arms` | GET/POST | Thompson Sampler arms |
| `/compose` | POST | Generate message |
| `/classify` | POST | Classify email |
| `/inbox` | GET | View classified emails |
| `/feedback` | POST | Record open/reply |
| **`/competitor-leads`** | GET | 🎯 Get leads using competitor networks |
| **`/webhook/email-events`** | POST | 🔄 Webhook for email events |
| **`/webhook/sendgrid`** | POST | 🔄 SendGrid Inbound Parse |

---

## Quick Commands

### Check System Health
```bash
curl https://admin.apexmediation.ee/api/autonomous/health
curl https://admin.apexmediation.ee/api/autonomous/stats
```

### View Templates
```bash
curl "https://admin.apexmediation.ee/api/autonomous/templates?type=email"
curl "https://admin.apexmediation.ee/api/autonomous/templates?type=followup"
```

### 🎯 Get Competitor Leads (NEW)
```bash
# Tier 1 competitors (IronSource, Unity, AppLovin)
curl "https://admin.apexmediation.ee/api/autonomous/competitor-leads?tier=1"

# IronSource users with high scores
curl "https://admin.apexmediation.ee/api/autonomous/competitor-leads?competitor=ironsource&minScore=60"
```

### Create Lead Source
```bash
curl -X POST https://admin.apexmediation.ee/api/autonomous/sources \
  -H "Content-Type: application/json" \
  -d '{"name":"US Games","sourceType":"itunes","searchQuery":"arcade","category":"Games","region":"us"}'
```

### Run Discovery
```bash
curl -X POST https://admin.apexmediation.ee/api/autonomous/discover \
  -H "Content-Type: application/json" \
  -d '{"sourceId":"<uuid>","batchSize":5}'
```

### Score Contacts
```bash
curl -X POST https://admin.apexmediation.ee/api/autonomous/score \
  -H "Content-Type: application/json" \
  -d '{"limit":50}'
```

### Compose Message
```bash
curl -X POST https://admin.apexmediation.ee/api/autonomous/compose \
  -H "Content-Type: application/json" \
  -d '{"contactId":"<uuid>","firstName":"John","company":"GameCo","appName":"PuzzleQuest","appGenre":"Games"}'
```

---

## Key Tables

| Table | Purpose |
|-------|---------|
| `sa_sources` | Lead discovery sources |
| `sa_contacts` | Contact records (+`has_competitor`, `top_competitor`) |
| `sa_heuristic_scores` | Lead scores (auto-calc tier) |
| `sa_spintax_templates` | Email templates |
| `sa_spintax_history` | Sent message history |
| `sa_bandit_arms` | A/B test variants |
| `sa_bandit_trials` | A/B test results |
| `sa_inbox_messages` | Classified emails |
| `sa_bayes_training` | Classifier training data |
| **`sa_outbound_messages`** | Sent emails (closed-loop tracking) |
| **`sa_email_events`** | Webhook events (opens, clicks, etc) |
| **`sa_competitor_detections`** | Per-contact competitor cache |

---

## Scoring Tiers

| Tier | Score | Priority |
|------|-------|----------|
| `hot` | 70-100 | 🔥 High (often has competitors) |
| `warm` | 50-69 | ✅ Good potential |
| `cold` | 30-49 | ❄️ Worth nurturing |
| `ice` | 0-29 | 🧊 Lowest priority |

---

## 🎯 Competitor Tiers

| Tier | Networks | Score Boost |
|------|----------|-------------|
| 1 | IronSource, Unity, AppLovin | +45-50 |
| 2 | Mintegral, Vungle, Chartboost, Fyber, InMobi | +30-35 |
| 3 | AdColony, Tapjoy, StartApp, Ogury | +20-25 |

---

## Classification Categories

| Category | Meaning | Auto-Action |
|----------|---------|-------------|
| `INTERESTED` | Positive reply | Follow-up queue |
| `NOT_INTERESTED` | Polite decline | Close |
| `UNSUBSCRIBE` | Opt-out | Remove |
| `OUT_OF_OFFICE` | Auto-reply | Re-queue later |
| `BOUNCE` | Delivery fail | Mark invalid |
| `UNKNOWN` | Uncertain | Human review |

---

## Debug Queries

```sql
-- Check sources
SELECT name, status, total_contacts_found FROM sa_sources;

-- Check scoring distribution
SELECT tier, COUNT(*) FROM sa_heuristic_scores GROUP BY tier;

-- Check template usage
SELECT name, times_used, reply_rate FROM sa_spintax_templates;

-- Check bandit convergence
SELECT arm_name, total_trials, empirical_rate FROM sa_bandit_arms;

-- Check pending reviews
SELECT * FROM sa_inbox_messages WHERE requires_human_review AND human_reviewed_at IS NULL;
```

---

## Server Access

```bash
# SSH to app server (control-plane runs here)
./scripts/ops/hetzner_ssh.sh app

# View logs
journalctl -u control-plane -f

# Restart service
sudo systemctl restart control-plane
```

---

## Related Docs

- [Full Documentation](AUTONOMOUS_SALES_V2.md)
- [V1 Cialdini System](SALES_AUTOMATION_SUMMARY.md)
- [V1 Operations](SALES_AUTOMATION_OPERATIONS.md)
