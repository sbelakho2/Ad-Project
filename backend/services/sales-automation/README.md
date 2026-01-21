# Automated Sales System (Zero-Cost)

## Overview

Fully automated outbound sales system for ApexMediation. **No paid API calls** - uses only free public data sources, self-hosted services, and existing infrastructure.

## Business Context

ApexMediation is a BYO (Bring-Your-Own) ad mediation platform targeting:
- **Mobile app publishers** monetizing with ads (gaming, utility, media apps)
- **Mid-tier publishers** ($10K-$500K/month in ad revenue)
- **Publishers frustrated** with opaque mediation platforms (Unity/ironSource/MAX)

## Revenue Model
- **Starter**: 0% (free up to $10K/app/month) - Loss leader for acquisition
- **Growth**: 2.5% of mediated revenue
- **Scale**: 2.0% → 1.25% with annual prepay
- **Enterprise**: 1.0-1.5% custom

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        AUTOMATED SALES ENGINE                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌────────────┐ │
│  │  PROSPECT    │ →  │   ENRICH &   │ →  │   SCORE &    │ →  │  OUTREACH  │ │
│  │  DISCOVERY   │    │   VALIDATE   │    │  PRIORITIZE  │    │  SEQUENCE  │ │
│  └──────────────┘    └──────────────┘    └──────────────┘    └────────────┘ │
│        ↓                    ↓                   ↓                  ↓        │
│  • App Store Scrape  • Domain Lookup    • ML Scoring         • Email       │
│  • GitHub/FOSS       • Company Size     • Intent Signals     • LinkedIn    │
│  • SensorTower Free  • Tech Stack       • Fit Assessment     • Follow-ups  │
│  • Public Lists      • Contact Find     • Revenue Est.       • Retargeting │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           DATA STORAGE (PostgreSQL)                          │
│  prospects │ contacts │ sequences │ touchpoints │ conversions │ analytics   │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Data Sources (All Free)

### 1. App Store Discovery
- **Google Play**: Public search API (no auth required)
- **Apple App Store**: RSS feeds + lookup API (no auth required)
- **Alternative stores**: Amazon, Samsung Galaxy Store

### 2. Company Enrichment  
- **WHOIS**: Domain registration info
- **DNS/MX**: Email provider detection
- **GitHub**: SDK usage, tech stack
- **Crunchbase (limited free)**: Funding, employee count
- **LinkedIn public profiles**: Job titles, company size

### 3. Contact Discovery
- **Email patterns**: {first}@domain.com, {first}.{last}@domain.com
- **Public GitHub profiles**: Developer emails from commits
- **Hunter.io free tier**: 25 searches/month
- **DNS MX verification**: Email deliverability check

### 4. Intent Signals
- **Job postings**: "ad monetization", "mediation", "SDK integration"
- **GitHub activity**: SDK repos, ad-related issues
- **App updates**: Frequent monetization changes
- **Review analysis**: Ad-related complaints

## Components

### 1. ProspectDiscoveryService
Finds potential customers from public sources.

### 2. LeadEnrichmentService  
Enriches prospects with company/contact data.

### 3. LeadScoringService
ML-free scoring using rule-based signals.

### 4. OutreachSequenceService
Manages email sequences with personalization.

### 5. SalesAnalyticsService
Tracks pipeline, conversions, and optimizes.

## Deployment

Runs on the Hetzner App Server (46.62.252.96) as a background service:

```bash
# 1. Build and deploy (from project root)
./scripts/ops/deploy-sales-automation.sh --migrate --restart

# 2. Check service status
ssh root@46.62.252.96 systemctl status sales-automation

# 3. View logs
ssh root@46.62.252.96 journalctl -u sales-automation -f
```

### CLI Commands

| Command | Description |
|---------|-------------|
| `discover` | Run prospect discovery from app stores |
| `enrich [n]` | Enrich up to n prospects (default 50) |
| `outreach` | Send scheduled emails |
| `enroll` | Enroll qualified contacts into sequences |
| `analytics` | Run daily roll-up |
| `full` | Run complete cycle |
| `pipeline` | Show pipeline metrics |
| `funnel` | Show conversion funnel |
| `sequences` | Show sequence performance |
| `trend [days]` | Show daily trend |
| `daemon` | Run as background service |

**Example:**
```bash
ssh root@46.62.252.96 'cd /opt/adproject/backend && node dist/src/scripts/salesAutomationCli.js pipeline'
```

### Tracking Endpoints

Add to Express app:
```typescript
import { createSalesTrackingRouter } from './routes/salesTracking';
app.use('/', createSalesTrackingRouter(pool));
```

Endpoints:
- `GET /track/open/:emailId` - Email open tracking pixel
- `GET /track/click/:emailId/:linkId?url=...` - Link click redirect
- `GET /unsubscribe/:contactId` - Unsubscribe page
- `POST /track/reply` - Reply webhook (from Resend)
- `POST /track/bounce` - Bounce webhook (from Resend)

## Scheduling

| Job | Schedule | Description |
|-----|----------|-------------|
| Prospect Discovery | Daily 2 AM | Scrape app stores for new prospects |
| Lead Enrichment | Hourly | Enrich queued prospects |
| Lead Scoring | Hourly | Score enriched leads |
| Outreach Send | Every 15 min | Send scheduled emails |
| Analytics Roll-up | Daily 6 AM | Calculate pipeline metrics |

## Legal Compliance

- GDPR: Legitimate interest basis for B2B prospecting
- CAN-SPAM: Unsubscribe link in all emails
- CCPA: Honor opt-out requests
- Rate limiting: Respect robots.txt and API limits
