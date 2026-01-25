# 🤖 Autonomous Sales V2 - Quick Reference

## API Endpoints (All at `https://admin.apexmediation.ee/api/autonomous/`)

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Health check |
| `/stats` | GET | System dashboard |
| `/sources` | GET/POST | Lead source management |
| `/discover` | POST | Run discovery job |
| `/score` | POST | Score contacts |
| `/templates` | GET/POST | Spintax templates |
| `/templates/preview` | POST | Preview variations |
| `/arms` | GET/POST | Thompson Sampler arms |
| `/compose` | POST | Generate message |
| `/classify` | POST | Classify email |
| `/inbox` | GET | View classified emails |
| `/feedback` | POST | Record open/reply |

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
  -d '{"sourceId":"<uuid>"}'
```

### Score Contacts
```bash
curl -X POST https://admin.apexmediation.ee/api/autonomous/score \
  -H "Content-Type: application/json" \
  -d '{"limit":100}'
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
| `sa_contacts` | Contact records |
| `sa_heuristic_scores` | Lead scores (auto-calc tier) |
| `sa_spintax_templates` | Email templates |
| `sa_spintax_history` | Sent message history |
| `sa_bandit_arms` | A/B test variants |
| `sa_bandit_trials` | A/B test results |
| `sa_inbox_messages` | Classified emails |
| `sa_bayes_training` | Classifier training data |

---

## Scoring Tiers

| Tier | Score | Priority |
|------|-------|----------|
| `whale` | 70-100 | 🔥 High |
| `qualified` | 40-69 | ✅ Medium |
| `cold` | 0-39 | ❄️ Low |

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
