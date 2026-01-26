# Sales Automation System - ACTUALLY USABLE

> **⚠️ Architecture Note (January 2026):**  
> This is the **V1 backend sales automation** (trial nurture, sequences, Resend integration) that runs on the **App Server backend**.
>
> For **Autonomous Sales V2** (outbound prospecting, Thompson Sampling, Spintax), see:
> - [AUTONOMOUS_SALES_V2.md](../../../docs/Internal/Sales/AUTONOMOUS_SALES_V2.md) - Full guide
> - V2 runs on the **Test Server** (157.90.126.205:4001)
> - The App Server control-plane is strictly a proxy client for V2

## 🎛️ Control Panel UI (NEW)

**All settings are now editable from the Control Plane UI:**
- Navigate to: `https://admin.apexmediation.ee/admin/sales-automation/control`

### What You Can Control From UI:
- ✅ Sender name and email (no code changes!)
- ✅ Daily send limits
- ✅ Enable/disable review queue (drafts)
- ✅ Require email verification before sending
- ✅ View and edit email sequences
- ✅ Approve/reject draft emails
- ✅ Run outreach cycles (dry-run or live)

## Quick Start (5 minutes)

```bash
# 1. Setup database tables and default sequences
cd backend
DATABASE_URL="..." RESEND_API_KEY="..." npx ts-node -P tsconfig.json services/sales-automation/cli.ts setup

# 2. Import leads from CSV
npx ts-node -P tsconfig.json services/sales-automation/cli.ts import leads.csv

# 3. Enroll contacts in a sequence
npx ts-node -P tsconfig.json services/sales-automation/cli.ts enroll "Mobile App" new 10

# 4. Send emails (dry-run first!)
npx ts-node -P tsconfig.json services/sales-automation/cli.ts run --dry-run

# 5. Actually send
npx ts-node -P tsconfig.json services/sales-automation/cli.ts run
```

## What This System ACTUALLY Does

### Reality vs. Theory

| Component | Status | Notes |
|-----------|--------|-------|
| Control Panel UI | ✅ WORKING | Full control from browser |
| DB-backed Config | ✅ WORKING | Sender settings in sa_config table |
| CLI Interface | ✅ WORKING | Import, enroll, send, track |
| Database Tables | ✅ CREATED | sa_contacts, sa_sequences, sa_enrollments, sa_emails, sa_config |
| Email Sending | ✅ WORKING | Via Resend (3K/month free) |
| Review Queue | ✅ WORKING | Optional draft approval before sending |
| Email Verification | ✅ WORKING | Optional requirement for verified contacts |
| Domain Health Guard | ✅ WORKING | Auto-stops at high bounce rates |
| Spam Prevention | ✅ WORKING | Pre-flight content check |
| Default Sequences | ✅ CREATED | 2 sequences ready to use |

### CSV Format

Required column: `email`  
Optional: `first_name`, `last_name`, `company_name`, `job_title`, `app_name`, `app_downloads`

```csv
email,first_name,company_name,app_name,app_downloads
john@company.com,John,Game Studio,Puzzle Master,100K+
```

### Default Sequences

1. **Mobile App Publisher - Ad Mediation** (3 emails over 8 days)
   - Day 0: Initial reach out about ad revenue optimization
   - Day 3: Follow-up with case study
   - Day 8: Final gentle nudge

2. **Simple Introduction** (2 emails over 4 days)
   - Day 0: Brief intro
   - Day 4: Follow-up

### Personalization Variables

Use these in your email templates:
- `{{first_name}}` - Contact's first name
- `{{company_name}}` - Company name
- `{{app_name}}` - App name
- `{{app_downloads}}` - Download count

## CLI Commands

```bash
# Setup
setup                           # Create tables and default sequences

# Contacts
import <file.csv>               # Import from CSV
add <email> [--name "..."]      # Add single contact
contacts [status]               # List contacts

# Sequences  
sequences                       # List available sequences

# Outreach
enroll <sequence> [status] [n]  # Enroll N contacts with status in sequence
run [--dry-run]                 # Send pending emails
status                          # Show stats

# Example full workflow
npx ts-node cli.ts import leads.csv
npx ts-node cli.ts enroll "Mobile App" new 10
npx ts-node cli.ts run --dry-run   # Preview
npx ts-node cli.ts run             # Send!
npx ts-node cli.ts status          # Check results
```

## Environment Variables

```bash
DATABASE_URL=postgresql://...     # Required
RESEND_API_KEY=re_...             # Required for sending
RESEND_WEBHOOK_SECRET=...          # Optional (signature verification)
SALES_AUTOMATION_ENABLED=true      # Enable autonomous engine
SALES_AUTOMATION_CRON=*/30 * * * * # Run every 30 minutes
SALES_AUTOMATION_RUN_ON_STARTUP=false
SALES_AUTOMATION_DEFAULT_MAILBOX=sales@apexmediation.ee
SALES_AUTOMATION_MAX_DAILY_EMAILS=100
SALES_AUTOMATION_SELF_LEARNING_ENABLED=true
SALES_AUTOMATION_AUTONOMOUS_MODE=true
```

## Automated Mode (Option B)

The autonomous engine is wired into the backend startup. It uses the main
sales automation schema created by migrations:

- `041_sales_automation_outbound.sql`
- `20250609_sales_automation_improvements.sql`
- `20250610_revolutionary_sales_features.sql`

Once migrations are applied and `SALES_AUTOMATION_ENABLED=true`, the scheduler
will run the full automation cycle on the configured cron.

## Webhook Setup (for tracking)

Add this route to your Express app:

```typescript
import { handleResendWebhook } from './services/sales-automation/webhook';

app.post('/api/webhooks/resend', express.raw({ type: 'application/json' }), async (req, res) => {
  const signature = req.headers['svix-signature'] as string || '';
  const result = await handleResendWebhook(req.body.toString(), signature);
  res.status(result.success ? 200 : 400).json(result);
});
```

Then configure in Resend dashboard:
- Webhook URL: `https://api.apexmediation.ee/api/webhooks/resend`
- Events: email.opened, email.clicked, email.bounced, email.complained

---

## Advanced Services (Need Data First)

The "revolutionary" services require data to be useful:

### Domain Health Guard
Monitors bounce rates and spam complaints. Useful after sending 50+ emails.

### Self-Learning Sequences  
A/B tests email variants. Needs 100+ sends to have statistical significance.

### Predictive Lead Scoring
Scores leads based on engagement. Needs historical conversion data.

### Smart Reply Analyzer
Categorizes replies. Needs incoming replies to analyze.

**Start simple with the CLI → Generate data → Then the advanced services become useful.**
- Daily sending limits
