# 🤖 Autonomous Sales System V2

**Date:** January 2026  
**Status:** ✅ Deployed to Production (v2.6 - Distributed Architecture)  
**Test Server API:** `http://203.0.113.11:4001/api/autonomous/` (internal)  
**App Server Proxy:** `https://admin.apexmediation.ee/api/autonomous/` (proxies to test server)

---

## � CRITICAL OPERATIONAL RULES - READ FIRST

> **These rules exist because of past incidents. Violating them WILL break production.**

### ❌ NEVER DO

| Don't | Why |
|-------|-----|
| **Modify App Server .env without permission** | App Server runs production services (backend, console, website, control-plane) |
| **Change `SALES_API_HOST` from `127.0.0.1`** | SSH tunnel already forwards localhost:4001 → Test Server. Changing this breaks the proxy. |
| **Kill processes on App Server** | Control-plane may appear as orphan process but is managed by systemd |
| **Touch port 4000, 4001, 3500 config on App Server** | These are tunneled to Test Server - the config is correct as-is |
| **Run sales automation code on App Server** | App Server is PROXY ONLY. All execution happens on Test Server. |

### ✅ ALWAYS DO

| Do | How |
|----|-----|
| **Test Server changes only** | All sales-api, edge-worker, inbound-smtp changes go to Test Server (203.0.113.11) |
| **Check tunnel health first** | `ssh app 'curl -s http://127.0.0.1:4001/health'` - if this fails, tunnel is down |
| **Read logs before changing** | `journalctl -u control-plane -n 50` on App Server shows proxy errors |
| **Verify after restart** | Always check `/api/autonomous/health` returns "healthy" status |

### 🔧 Quick Reference - Server Roles

```
APP SERVER (203.0.113.10)     TEST SERVER (203.0.113.11)     DB SERVER (203.0.113.12)
├── control-plane:4000        ├── sales-api:4001 (PM2)         └── PostgreSQL (via tunnel)
│   └── PROXY ONLY            │   └── ALL EXECUTION
├── backend (PM2)             ├── edge-worker:3500 (systemd)
├── console (PM2)             │   └── ML models
├── website (PM2)             └── inbound-smtp:2525
└── SSH tunnels to Test       └── db-tunnel to App→DB
```

### 🛠️ Recovery Commands

```bash
# If control-plane won't start (EADDRINUSE):
ssh app 'lsof -ti :4000 | xargs -r kill -9 && systemctl start control-plane'

# If sales automation shows unhealthy - check tunnel:
ssh app 'curl -s http://127.0.0.1:4001/health'  # Should return {"status":"ok"}

# If tunnel is down, restart it:
ssh app 'systemctl restart sales-api-tunnel'

# Verify full health:
ssh app 'curl -s http://localhost:4000/api/autonomous/health'
```

---

## �📋 Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Core Modules](#core-modules)
4. [Competitor Targeting (Sniper)](#competitor-targeting-sniper)
5. [Closed-Loop Learning](#closed-loop-learning)
6. [API Reference](#api-reference)
7. [Database Schema](#database-schema)
8. [Spintax Templates](#spintax-templates)
9. [Thompson Sampling](#thompson-sampling)
10. [Lead Scoring](#lead-scoring)
11. [Email Classification](#email-classification)
12. [Operations Guide](#operations-guide)
13. [Troubleshooting](#troubleshooting)

---

## Overview

### What Is This?

The Autonomous Sales System V2 is a **zero-cost**, self-improving outbound sales automation platform that discovers, qualifies, and engages app developers without any paid APIs or services.

### Design Principles

1. **Zero External Costs**: No paid APIs (OpenAI, Clearbit, etc.) - uses heuristics and free data
2. **Self-Optimizing**: Thompson Sampling learns which subject lines convert best
3. **Anti-Spam**: Levenshtein distance ensures unique message variations
4. **Human-in-the-Loop**: Naive Bayes classifier routes uncertain emails for review

### Key Capabilities

| Capability | Module | Description |
|------------|--------|-------------|
| Lead Discovery | Hunter | Scrapes app stores for contacts |
| Lead Scoring | Analyst | Heuristic scoring (app-ads.txt, genre, etc.) |
| **Competitor Targeting** | Analyst | "Sniper" mode: detects IronSource, Unity, AppLovin users |
| Message Personalization | Spintax Engine | {option1\|option2} syntax with uniqueness checks |
| Subject Line A/B Testing | Thompson Sampler | Multi-armed bandit optimization |
| Email Classification | Clerk | Naive Bayes + rule-based classification |
| **Closed-Loop Learning** | Webhooks | Auto-updates Sampler from email responses (Homespun SMTP) |
| Orchestration | Orchestrator | Coordinates all modules |

### Comparison to V1 (Cialdini System)

| Aspect | V1 (Cialdini) | V2 (Autonomous) |
|--------|---------------|-----------------|
| Focus | Trial nurture (inbound) | Outbound prospecting |
| Targeting | Existing signups | Cold app developers |
| Discovery | Manual/import | Automated scraping |
| Optimization | Static sequences | Thompson Sampling A/B |
| External APIs | OpenAI, Resend | None (zero-cost) |
| Location | Backend | **Test Server** (App Server is proxy only) |

---

## Architecture

### Distributed Deployment (v2.6 - January 2026)

The system uses a **distributed architecture** where the **Test Server runs ALL sales automation work** and the **App Server is strictly a proxy client**:

| Server | IP | Role | Services |
|--------|-----|------|----------|
| **App Server** | 203.0.113.10 | **Proxy Client ONLY** | control-plane (port 4000) proxies to test server |
| **Test Server** | 203.0.113.11 | **Execution Server** | sales-api (port 4001), edge-worker (port 3500), inbound-smtp |
| **DB Server** | 203.0.113.12 | Database | PostgreSQL (NOT directly accessible) |

> **⚠️ CRITICAL: App Server runs NO sales automation logic.** It is strictly a proxy client.  
> All orchestration, spintax processing, scoring, and messaging runs on the Test Server.

```
┌─────────────────────────────────────────────────────────────────────┐
│                   APP SERVER (203.0.113.10)                          │
│                      *** PROXY CLIENT ONLY ***                       │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   control-plane (Port 4000)                                          │
│     └── /api/autonomous/* routes → PROXY to Test Server              │
│     └── NO orchestrator, NO spintax engine, NO sales logic          │
│                                                                      │
│   sales-api-tunnel.service (systemd)                                 │
│     └── SSH tunnel: localhost:4001 → Test Server:4001                │
│                                                                      │
│   edge-tunnel.service (systemd)                                      │
│     └── SSH tunnel: localhost:3500 → Test Server:3500                │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
                            │
                            │ SSH Tunnels (secure)
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│                  TEST SERVER (203.0.113.11)                          │
│                   *** ALL SALES AUTOMATION RUNS HERE ***             │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   sales-api (PM2, Port 4001)                                         │
│     ├── Autonomous Orchestrator                                      │
│     ├── Thompson Sampler (multi-armed bandit)                        │
│     ├── Spintax Engine (message personalization)                     │
│     ├── Hunter (lead discovery)                                      │
│     ├── Analyst (lead scoring + competitor targeting)                │
│     └── Clerk (email classification)                                 │
│                                                                      │
│   edge-worker.service (systemd, Port 3500)                           │
│     ├── Icebreaker Engine (LaMini-Flan-T5-248M)                      │
│     ├── Hybrid Classifier (Naive Bayes + DistilBERT)                 │
│     ├── Reputation Analyzer                                          │
│     └── Playwright Scraper                                           │
│                                                                      │
│   inbound-smtp.service (systemd, Port 2525)                          │
│     └── Receives reply emails → POSTs to /webhook/sendgrid          │
│                                                                      │
│   db-tunnel.service (systemd)                                        │
│     └── SSH tunnel: localhost:6432 → App Server PgBouncer:6432       │
│                                                                      │
│   Memory: ~1.5GB total (ML models + sales-api)                       │
│   Authentication: X-API-Key header                                   │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
                            │
                            │ SSH Tunnel via App Server
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    DB SERVER (203.0.113.12)                          │
│                    *** NOT DIRECTLY ACCESSIBLE ***                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   PostgreSQL 16 (Port 5432)                                          │
│     └── Only reachable via App Server PgBouncer (6432)               │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Proxy Architecture (App Server)

The App Server control-plane uses a simple proxy router that forwards all `/api/autonomous/*` requests to the Test Server:

```typescript
// control-plane/src/routes/autonomous-api-proxy.ts
// App Server is STRICTLY A CLIENT - no sales logic here

const SALES_API_BASE = `http://127.0.0.1:4001`; // via SSH tunnel

async function proxyToTestServer(req, res, path) {
  const url = `${SALES_API_BASE}/api/autonomous${path}`;
  const response = await fetch(url, {
    method: req.method,
    headers: { 'X-API-Key': SALES_API_KEY, ...req.headers },
    body: req.body ? JSON.stringify(req.body) : undefined,
  });
  res.status(response.status).json(await response.json());
}
```

### Sales API Server Architecture (Test Server)

```
┌─────────────────────────────────────────────────────────────────────┐
│                    SALES-API (Port 4001)                             │
│                      Test Server Only                                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                    Autonomous API Router                      │    │
│  │              /api/autonomous/*                                │    │
│  │  Batch limits: MAX_DISCOVER=10, MAX_SCORE=100                 │    │
│  └─────────────────────────────────────────────────────────────┘    │
│         │              │                              ▲              │
│         │              │                              │              │
│         │              │                    ┌─────────┴────────┐    │
│         │              │                    │  WEBHOOKS        │    │
│         │              │                    │  /webhook/sendgrid│   │
│         │              │                    │  /webhook/email-  │   │
│         │              │                    │   events          │   │
│         │              │                    └──────────────────┘    │
│         │              ▼                              │              │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                 AUTONOMOUS ORCHESTRATOR                       │    │
│  │    Coordinates all modules, manages workflows                 │    │
│  │    saveOutboundMessage() for closed-loop tracking             │    │
│  └─────────────────────────────────────────────────────────────┘    │
│         │              │              │              │               │
│         ▼              ▼              ▼              ▼               │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐        │
│  │  HUNTER   │  │  ANALYST  │  │  SPINTAX  │  │  CLERK    │        │
│  │ Discovery │  │  Scoring  │  │  Engine   │  │ Classifier│        │
│  │           │  │ + SNIPER  │  │           │  │           │        │
│  └───────────┘  └───────────┘  └───────────┘  └───────────┘        │
│         │              │              │              │               │
│         │              │              ▼              │               │
│         │              │       ┌───────────┐        │               │
│         │              │       │ THOMPSON  │◄───────┘               │
│         │              │       │  SAMPLER  │ ← Closed-loop feedback │
│         │              │       └───────────┘                        │
│         │              │              │                              │
│         └──────────────┴──────────────┴──────────────────────────────│
│                              │                                       │
│                              ▼                                       │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │              PostgreSQL (via SSH Tunnel to App Server)        │    │
│  │   DB Tunnel: localhost:6432 → App Server:6432 → DB:5432       │    │
│  │                                                               │    │
│  │   sa_sources, sa_contacts, sa_heuristic_scores,              │    │
│  │   sa_spintax_templates, sa_bandit_arms, sa_inbox_messages,   │    │
│  │   sa_outbound_messages, sa_email_events,                     │    │
│  │   sa_competitor_detections                                    │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘

        ┌────────────────────────────────────────────────────────┐
        │                  CLOSED-LOOP FLOW                       │
        │                                                         │
        │  1. Compose: Thompson Sampler picks subject arm         │
        │  2. Send: saveOutboundMessage() records arm + contact   │
        │  3. Event: Webhook receives open/click/reply/bounce     │
        │  4. Learn: recordOutcome(arm, success) updates Sampler  │
        │  5. Repeat: Next email uses updated probabilities       │
        │                                                         │
        └────────────────────────────────────────────────────────┘
```

### File Structure

**Test Server (`/opt/adproject/sales-automation/`):**
```
sales-automation/
├── dist/
│   └── index.js                   # Sales API server entry point
├── api.env                        # Environment config
├── ecosystem.api.config.js        # PM2 config
└── services/
    ├── autonomous-orchestrator.js # Main coordinator
    ├── math/
    │   └── thompson-sampler.js    # Multi-armed bandit
    ├── messaging/
    │   └── spintax-engine.js      # Message personalization
    ├── discovery/
    │   ├── hunter.js              # Lead discovery
    │   └── analyst.js             # Lead scoring
    └── inbox/
        └── clerk.js               # Email classification
```

**App Server (`/opt/adproject/control-plane/`):**
```
control-plane/src/
├── routes/
│   └── autonomous-api-proxy.ts    # PROXY ONLY - forwards to test server
├── services/
│   └── (NO sales automation services - proxy mode)
└── index.ts                       # NO orchestrator initialization
├── services/
│   ├── autonomous-orchestrator.ts # Main coordinator
│   ├── math/
│   │   └── thompson-sampler.ts    # Multi-armed bandit
│   ├── messaging/
│   │   └── spintax-engine.ts      # Message personalization
│   ├── discovery/
│   │   ├── hunter.ts              # Lead discovery
│   │   └── analyst.ts             # Lead scoring
│   └── inbox/
│       └── clerk.ts               # Email classification
└── tests/
    └── autonomous-sales.test.ts   # Simulation tests
```

---

## Core Modules

### 1. Hunter (Lead Discovery)

**File:** `control-plane/src/services/discovery/hunter.ts`

**Purpose:** Discovers app developer contacts from public sources.

**Data Sources:**
- iTunes Search API (free, no auth required)
- Google Play scraping (optional, via Playwright)
- App-ads.txt files (direct HTTP fetch)

**Key Functions:**

```typescript
// Search iTunes for apps by genre
searchITunesApps(params: {
  term?: string;
  genreId?: number;
  country?: string;
  limit?: number;
}): Promise<AppResult[]>

// Check app-ads.txt for ad network presence
checkAppAdsTxt(domain: string): Promise<{
  hasAdsTxt: boolean;
  content?: string;
  networks: string[];
}>

// Run discovery job for a source
runDiscoveryJob(sourceId: string): Promise<DiscoveryResult>
```

**iTunes Genre IDs (Most Valuable):**
- `6014` - Games
- `6016` - Entertainment
- `6017` - Education
- `6018` - Books
- `6002` - Utilities

### 2. Analyst (Lead Scoring)

**File:** `control-plane/src/services/discovery/analyst.ts`

**Purpose:** Scores leads using heuristic signals (no paid enrichment APIs).

**Scoring Signals:**

| Signal | Weight | Max Score | Description |
|--------|--------|-----------|-------------|
| `ads_txt_present` | 25% | 100 | Has app-ads.txt file |
| `competitor_detected` | 20% | 100 | **Uses competitor network (Sniper)** |
| `ad_network_count` | 15% | 100 | Number of ad networks |
| `popular_network` | 15% | 100 | Google, Facebook, Unity, etc. |
| `app_genre_fit` | 10% | 100 | Games > Entertainment > Business |
| `update_velocity` | 10% | 100 | Recent app updates |
| `has_email` | 10% | 100 | Discoverable contact |
| `has_website` | 5% | 100 | Professional presence |

**Tier Classification:**

| Tier | Score Range | Description |
|------|-------------|-------------|
| `hot` | 70-100 | High-value, often uses competitor networks |
| `warm` | 50-69 | Good potential, some positive signals |
| `cold` | 30-49 | Low signals, worth nurturing |
| `ice` | 0-29 | Minimal signals, lowest priority |

**Key Functions:**

```typescript
// Calculate score for a contact
calculateLeadScore(contact: Contact): LeadScore

// Detect competitors from app-ads.txt content
detectCompetitorsFromAdsTxt(adsTxtContent: string): DetectedCompetitor[]

// Save competitor detections to database
saveCompetitorDetectionsFromList(
  contactId: string, 
  competitors: DetectedCompetitor[]
): Promise<void>

// Save score to database
saveLeadScore(score: LeadScore): Promise<void>

// Get contacts needing scoring
getContactsNeedingScoring(limit?: number): Promise<Contact[]>
```

### 3. Spintax Engine (Message Personalization)

**File:** `control-plane/src/services/messaging/spintax-engine.ts`

**Purpose:** Generates unique email variations using spintax syntax.

**Spintax Syntax:**
```
{Hi|Hello|Hey} {{first_name}},

{I noticed|I came across|I saw} {{app_name}} on the {App Store|Play Store}...
```

**Features:**
- `{option1|option2|option3}` - Random selection
- `{{variable}}` - Variable substitution
- Levenshtein distance check ensures uniqueness
- History tracking prevents duplicate sends

**Key Functions:**

```typescript
// Spin and personalize a template
spinAndPersonalize(
  subjectSpintax: string,
  bodySpintax: string,
  context: PersonalizationContext
): SpinResult

// Generate unique variation (with anti-spam check)
generateUnique(
  subjectSpintax: string,
  bodySpintax: string,
  context: PersonalizationContext,
  maxAttempts?: number
): SpinResult

// Calculate Levenshtein distance
levenshteinDistance(a: string, b: string): number
```

### 4. Thompson Sampler (A/B Testing)

**File:** `control-plane/src/services/math/thompson-sampler.ts`

**Purpose:** Multi-armed bandit for optimizing subject lines.

**How It Works:**

1. **Beta Distribution**: Each subject line arm has α (successes) and β (failures)
2. **Thompson Sampling**: Sample from Beta(α, β) to pick winner
3. **Exploration/Exploitation**: Naturally balances trying new vs. known-good
4. **Convergence**: As trials increase, best performer wins more often

**Mathematical Foundation:**

```
Score ~ Beta(α, β) where:
  α = successes + 1 (prior)
  β = failures + 1 (prior)

Selection: Pick arm with highest sampled score
Update: On success → α++, On failure → β++
```

**Key Functions:**

```typescript
// Sample from Beta distribution
sampleBeta(alpha: number, beta: number): number

// Select best arm via Thompson Sampling
sampleAndSelect(armType: string): Promise<SamplingResult | null>

// Record outcome (success/failure)
recordOutcome(armId: string, success: boolean): Promise<void>

// Get bandit statistics
getBanditStats(armType: string): Promise<BanditStats>
```

### 5. Clerk (Email Classification)

**File:** `control-plane/src/services/inbox/clerk.ts`

**Purpose:** Classifies incoming emails using Naive Bayes + rules.

**Classification Categories:**

| Category | Description | Auto-Action |
|----------|-------------|-------------|
| `INTERESTED` | Positive response | Queue for follow-up |
| `NOT_INTERESTED` | Polite decline | Mark as closed |
| `UNSUBSCRIBE` | Opt-out request | Remove from list |
| `OUT_OF_OFFICE` | Auto-reply | Re-queue later |
| `BOUNCE` | Delivery failure | Mark invalid |
| `UNKNOWN` | Uncertain | Queue for human review |

**Rule-Based Patterns (Fast Path):**

```typescript
const RULE_PATTERNS = {
  UNSUBSCRIBE: [/unsubscribe/i, /remove.*list/i, /stop.*email/i],
  OUT_OF_OFFICE: [/out of office/i, /on vacation/i, /away from/i],
  BOUNCE: [/delivery.*failed/i, /undeliverable/i, /mailbox.*full/i],
  INTERESTED: [/sounds interesting/i, /tell me more/i, /schedule.*call/i],
  NOT_INTERESTED: [/not interested/i, /no thank/i, /please remove/i],
};
```

**Naive Bayes (Fallback):**
- Trained on labeled examples in `sa_bayes_training`
- Uses word frequency + Laplace smoothing
- Requires human review when confidence < 70%

**Key Functions:**

```typescript
// Classify an email
classifyEmail(params: {
  subject: string;
  body: string;
  fromEmail: string;
}): Promise<ClassificationResult>

// Submit human review (also updates Thompson Sampler for closed-loop)
submitReview(messageId: string, correctCategory: string): Promise<void>

// Get classification stats
getClassificationStats(): Promise<ClassificationStats>
```

---

## Competitor Targeting (Sniper) 🎯

The "Sniper" upgrade enables hyper-targeted outreach to developers already using competitor ad networks.

### Why It Works

Developers using IronSource, Unity Ads, or AppLovin are:
1. **Already ad-monetizing** (no education needed)
2. **Familiar with mediation** (faster sales cycle)
3. **Likely dissatisfied** (or they'd stick with one network)

### Target Competitors

| Tier | Networks | Score Boost | Priority |
|------|----------|-------------|----------|
| 1 | IronSource, Unity Ads, AppLovin MAX | +45-50 | Highest |
| 2 | Mintegral, Vungle, Chartboost, Fyber, InMobi | +30-35 | Medium |
| 3 | AdColony, Tapjoy, StartApp, Ogury | +20-25 | Lower |

### Detection Flow

```
1. Hunter scrapes app → gets app-ads.txt URL
2. Analyst calls detectCompetitorsFromAdsTxt()
3. Each detected competitor → saved to sa_competitor_detections
4. Contact flagged: has_competitor=true, top_competitor=<name>
5. Score boosted by competitor tier boost value
```

### API Endpoint

```
GET /api/autonomous/competitor-leads?tier=1&minScore=50
Response: {
  "leads": [...],
  "byCompetitor": { "IronSource": [...], "Unity Ads": [...] },
  "competitorStats": [{ "competitor": "IronSource", "count": 8, "avgScore": 82 }]
}
```

---

## Closed-Loop Learning 🔄

The system automatically learns from email responses to optimize subject lines over time.

### How It Works

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   COMPOSE    │     │    SEND      │     │   WEBHOOK    │
│              │     │              │     │              │
│ Thompson     │     │ Save to      │     │ Receive      │
│ Sampler      │────▶│ sa_outbound_ │────▶│ open/click/  │
│ picks arm    │     │ messages     │     │ reply/bounce │
└──────────────┘     └──────────────┘     └──────────────┘
                                                 │
                                                 ▼
                           ┌──────────────────────────────────────┐
                           │         RECORD OUTCOME               │
                           │                                      │
                           │  Open/Click/Reply → recordOutcome(   │
                           │                      arm, true)      │
                           │  Bounce/Unsubscribe → recordOutcome( │
                           │                        arm, false)   │
                           └──────────────────────────────────────┘
                                                 │
                                                 ▼
                           ┌──────────────────────────────────────┐
                           │         THOMPSON SAMPLER             │
                           │                                      │
                           │  Alpha++ for successes               │
                           │  Beta++ for failures                 │
                           │  Next email more likely to use       │
                           │  winning subject lines               │
                           └──────────────────────────────────────┘
```

### Webhook Endpoints

| Endpoint | Purpose |
|----------|---------|
| `/webhook/email-events` | Generic events (works with any ESP) |
| `/webhook/sendgrid` | Self-hosted Inbound Parse (mimics SendGrid format) |

### Event to Outcome Mapping

| Event | Outcome | Effect on Arm |
|-------|---------|---------------|
| `open` | Success | α += 1 |
| `click` | Success | α += 1 |
| `reply` | Success | α += 1 |
| `bounce` | Failure | β += 1 |
| `unsubscribe` | Failure | β += 1 |

### Key Tables

| Table | Purpose |
|-------|---------|
| `sa_outbound_messages` | Tracks sent emails with `subject_arm_id` |
| `sa_email_events` | Stores webhook events with `outcome_recorded` flag |

---

## API Reference

### Base URL
```
https://admin.apexmediation.ee/api/autonomous
```

### Endpoints

#### Health Check
```
GET /health
Response: { "status": "healthy", "service": "autonomous-sales", "timestamp": "..." }
```

#### System Statistics
```
GET /stats
Response: {
  "success": true,
  "stats": {
    "discovery": { "sources": 0, "lastScrape": null },
    "scoring": { "contacts": 3, "scored": 3, "avgScore": 50 },
    "optimizer": { "arms": 0, "trials": 0, "convergence": 0 },
    "inbox": { "total": 0, "pendingReview": 0 }
  }
}
```

#### Lead Sources
```
GET /sources
Response: { "success": true, "sources": [...] }

POST /sources
Body: { "name": "...", "sourceType": "itunes", "searchQuery": "...", "category": "...", "region": "us" }
Response: { "success": true, "sourceId": "uuid" }
```

#### Discovery
```
POST /discover
Body: { "sourceId": "uuid" }
Response: { "success": true, "discovered": 50, "source": {...} }
```

#### Scoring
```
POST /score
Body: { "contactId": "uuid" } or { "limit": 50 }
Response: { "success": true, "scored": 50, "byTier": { "whale": 5, "qualified": 20, "cold": 25 } }
```

#### Templates
```
GET /templates?type=email
Response: { "success": true, "templates": [...] }

POST /templates
Body: { "name": "...", "templateType": "email", "subjectSpintax": "...", "bodySpintax": "..." }
Response: { "success": true, "templateId": "uuid" }

POST /templates/preview
Body: { "subjectSpintax": "...", "bodySpintax": "...", "count": 5 }
Response: { "success": true, "variations": [...] }
```

#### Bandit Arms
```
GET /arms?type=subject_line
Response: { "success": true, "arms": [...], "stats": {...} }

POST /arms
Body: { "armType": "subject_line", "armName": "...", "armValue": "..." }
Response: { "success": true, "armId": "uuid" }
```

#### Message Composition
```
POST /compose
Body: { "contactId": "uuid", "firstName": "...", "company": "...", "appName": "...", "appGenre": "..." }
Response: {
  "success": true,
  "message": {
    "subject": "Question about AppName monetization",
    "body": "Hi John, I noticed AppName...",
    "templateId": "uuid",
    "subjectArmId": "uuid",
    "variationHash": "abc123"
  }
}
```

#### Email Classification
```
GET /inbox?status=pending_review
Response: { "success": true, "messages": [...], "stats": {...} }

POST /classify
Body: { "subject": "...", "body": "...", "fromEmail": "..." }
Response: {
  "success": true,
  "classification": {
    "category": "INTERESTED",
    "confidence": 0.87,
    "method": "naive_bayes",
    "requiresReview": false
  }
}
```

#### Feedback (Learning)
```
POST /feedback
Body: { "armId": "uuid", "eventType": "open" | "reply" | "bounce" }
Response: { "success": true }
```

### Setup Guide (Zero-Cost Cloudflare Integration)

To enable closed-loop learning for **free** (no monthly fees), we use **Cloudflare Email Workers**. This allows you to receive emails on a subdomain and forward them to the Autonomous System as JSON.

#### Step 1: Choose a Subdomain
Separate your automation traffic from your main inbox.
- **Recommended:** `replies.apexmediation.ee` or `sales.apexmediation.ee`

#### Step 2: Configure Cloudflare Email Routing
1. Go to Cloudflare Dashboard > **Email** > **Email Routing**.
2. Enable Email Routing (this will maintain the necessary MX records for you).
3. Create a **Custom Address**: `*@replies.apexmediation.ee` (catch-all).

#### Step 3: Create the Email Worker
1. Go to **Workers & Pages** > **Create Application** > **Create Worker**.
2. Name it `sales-reply-parser`.
3. Paste the following code (uses `postal-mime` to handle the parsing):

```javascript
import PostalMime from 'postal-mime';

export default {
  async email(message, env, ctx) {
    const parser = new PostalMime();
    const email = await parser.parse(message.raw);
    
    // Cloudflare -> SendGrid-format Adapter
    const payload = {
      from: message.from,
      to: message.to,
      subject: message.headers.get('subject') || '',
      text: email.text || '',
      html: email.html || ''
    };

    // Forward to Control Plane
    await fetch('https://admin.apexmediation.ee/api/autonomous/webhook/sendgrid', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
  }
};
```

#### Step 4: Route Email to Worker
1. Go back to **Email Routing** > **Routes**.
2. Create a rule:
   - **Action:** Send to a Worker
   - **Worker:** `sales-reply-parser`
   - **Match:** All emails to `replies.apexmediation.ee`

#### Step 5: Verification
Send an email to `test@replies.apexmediation.ee`.
1. Cloudflare receives it instantly (zero cost).
2. Worker parses it and POSTs it to your server.
3. Check logs: `journalctl -u control-plane -f` → `SendGrid webhook received` (the adapter mimics the format).

---

## Database Schema

**Migration:** `backend/migrations/049_autonomous_sales_v2.sql`

### Tables

#### sa_sources
Lead discovery sources configuration.

```sql
CREATE TABLE sa_sources (
  id UUID PRIMARY KEY,
  name VARCHAR(255) NOT NULL,
  source_type VARCHAR(50) NOT NULL,  -- 'itunes', 'google_play', 'manual'
  search_query TEXT,
  category VARCHAR(100),
  region VARCHAR(10) DEFAULT 'us',
  status VARCHAR(20) DEFAULT 'active',  -- 'active', 'paused', 'archived'
  last_scraped_at TIMESTAMP,
  total_contacts_found INTEGER DEFAULT 0,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### sa_heuristic_scores
Lead scoring with auto-calculated tier.

```sql
CREATE TABLE sa_heuristic_scores (
  id UUID PRIMARY KEY,
  contact_id UUID REFERENCES sa_contacts(id),
  
  -- Individual scores (0-100)
  app_ads_txt_score INTEGER DEFAULT 0,
  sdk_diversity_score INTEGER DEFAULT 0,
  update_velocity_score INTEGER DEFAULT 0,
  review_velocity_score INTEGER DEFAULT 0,
  tech_sophistication_score INTEGER DEFAULT 0,
  company_size_score INTEGER DEFAULT 0,
  
  -- Raw signals
  signals JSONB DEFAULT '{}',
  
  -- Auto-calculated (GENERATED ALWAYS)
  overall_score INTEGER GENERATED ALWAYS AS (...) STORED,
  tier VARCHAR(20) GENERATED ALWAYS AS (...) STORED,  -- 'whale', 'qualified', 'cold'
  
  last_analyzed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### sa_spintax_templates
Email templates with spintax syntax.

```sql
CREATE TABLE sa_spintax_templates (
  id UUID PRIMARY KEY,
  name VARCHAR(255) NOT NULL,
  description TEXT,
  template_type VARCHAR(50) DEFAULT 'email',  -- 'email', 'subject', 'followup'
  subject_spintax TEXT NOT NULL,
  body_spintax TEXT NOT NULL,
  available_variables JSONB DEFAULT '["first_name", "company_name", "app_name"]',
  
  -- Statistics
  times_used INTEGER DEFAULT 0,
  total_opens INTEGER DEFAULT 0,
  total_replies INTEGER DEFAULT 0,
  open_rate DECIMAL(5,2) GENERATED ALWAYS AS (...) STORED,
  reply_rate DECIMAL(5,2) GENERATED ALWAYS AS (...) STORED,
  
  active BOOLEAN DEFAULT true,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### sa_bandit_arms
Thompson Sampler arms for A/B testing.

```sql
CREATE TABLE sa_bandit_arms (
  id UUID PRIMARY KEY,
  arm_type VARCHAR(50) NOT NULL,  -- 'subject_line', 'template', 'send_time'
  arm_name VARCHAR(255) NOT NULL,
  arm_value TEXT NOT NULL,
  
  -- Beta distribution parameters
  alpha INTEGER DEFAULT 1,  -- Successes + prior
  beta INTEGER DEFAULT 1,   -- Failures + prior
  
  -- Statistics
  total_trials INTEGER DEFAULT 0,
  total_successes INTEGER DEFAULT 0,
  empirical_rate DECIMAL(5,4) GENERATED ALWAYS AS (...) STORED,
  
  active BOOLEAN DEFAULT true,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### sa_inbox_messages
Incoming email classification.

```sql
CREATE TABLE sa_inbox_messages (
  id UUID PRIMARY KEY,
  from_email VARCHAR(255) NOT NULL,
  subject TEXT,
  body_text TEXT,
  
  -- Classification
  classification VARCHAR(30),  -- 'INTERESTED', 'NOT_INTERESTED', etc.
  classification_confidence DECIMAL(5,2),
  classification_method VARCHAR(30),  -- 'rule_based', 'naive_bayes'
  
  -- Review
  requires_human_review BOOLEAN DEFAULT false,
  human_reviewed_at TIMESTAMP,
  reviewed_by VARCHAR(255),
  
  received_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### sa_bayes_training
Training data for Naive Bayes classifier.

```sql
CREATE TABLE sa_bayes_training (
  id UUID PRIMARY KEY,
  label VARCHAR(30) NOT NULL,  -- Category label
  body_text TEXT NOT NULL,     -- Training example
  source VARCHAR(50) DEFAULT 'manual',  -- 'manual', 'human_review', 'seed'
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### sa_outbound_messages (Closed-Loop Tracking)
Tracks sent emails with Thompson Sampler arm IDs for closed-loop learning.

```sql
CREATE TABLE sa_outbound_messages (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID NOT NULL REFERENCES sa_contacts(id),
  
  -- Content
  subject TEXT NOT NULL,
  body_text TEXT NOT NULL,
  body_html TEXT,
  
  -- Thompson Sampler tracking (KEY FOR CLOSED LOOP)
  subject_arm_id UUID REFERENCES sa_bandit_arms(id),
  template_id UUID,
  variation_hash VARCHAR(64),
  
  -- Status progression
  status VARCHAR(20) DEFAULT 'pending',  -- pending → sent → delivered → opened → clicked → replied
  sent_at TIMESTAMP,
  delivered_at TIMESTAMP,
  opened_at TIMESTAMP,
  clicked_at TIMESTAMP,
  replied_at TIMESTAMP,
  
  -- External tracking
  message_id VARCHAR(255),  -- SendGrid/Mailgun message ID
  provider VARCHAR(50),     -- sendgrid, resend, mailgun
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
-- Indexes: contact_id, subject_arm_id, status, sent_at
```

#### sa_email_events (Webhook Events)
Stores all webhook events for analytics and debugging.

```sql
CREATE TABLE sa_email_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  outbound_message_id UUID REFERENCES sa_outbound_messages(id),
  contact_id UUID REFERENCES sa_contacts(id),
  
  -- Event details
  event_type VARCHAR(50) NOT NULL,  -- open, click, bounce, unsubscribe, reply
  event_data JSONB DEFAULT '{}',
  
  -- Bandit feedback
  arm_id UUID REFERENCES sa_bandit_arms(id),
  outcome_recorded BOOLEAN DEFAULT false,
  
  -- Source
  source VARCHAR(50),  -- sendgrid, mailgun, manual, webhook
  raw_payload JSONB,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
-- Indexes: outbound_message_id, contact_id, event_type, arm_id
```

#### sa_competitor_detections (Sniper Targeting)
Caches detected competitors for quick filtering.

```sql
CREATE TABLE sa_competitor_detections (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID NOT NULL REFERENCES sa_contacts(id),
  
  -- Competitor info
  competitor_domain VARCHAR(255) NOT NULL,
  competitor_name VARCHAR(100),
  competitor_tier INTEGER DEFAULT 3,  -- 1=highest priority, 3=lowest
  
  -- Detection source
  detected_in VARCHAR(50) DEFAULT 'app_ads_txt',
  detection_confidence INTEGER DEFAULT 100,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE(contact_id, competitor_domain)
);
-- Indexes: contact_id, competitor_domain, competitor_tier
```

---

## Spintax Templates

### Seeded Templates

The system comes with 3 pre-configured templates:

#### 1. Initial Outreach - Ad Mediation
```
Subject: {Quick question about|Question re:|Regarding} {{app_name}} {monetization|ad revenue|ad strategy}

Body: {Hi|Hello|Hey} {{first_name}},

{I noticed|I came across|I saw} {{app_name}} on the {App Store|Play Store|app stores}...
[Full template in database]
```

#### 2. Follow-up #1 - Value Add
```
Subject: {Re: |Following up: |Quick follow-up on }{{app_name}}

Body: {Hi|Hey} {{first_name}},

{Just wanted to follow up|Circling back|Bumping this up}...
```

#### 3. Follow-up #2 - Final
```
Subject: {Last attempt|Final check-in|Closing the loop}: {{app_name}}

Body: {{first_name}},

{I'll keep this short|Quick one|Last message from me on this}...
```

### Creating Custom Templates

```bash
curl -X POST https://admin.apexmediation.ee/api/autonomous/templates \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Game Developer Special",
    "description": "Optimized for mobile game developers",
    "templateType": "email",
    "subjectSpintax": "{Boosting|Maximizing|Optimizing} {{app_name}} {ad revenue|monetization}",
    "bodySpintax": "Hey {{first_name}},\n\n{Love what you built with|Big fan of|Impressed by} {{app_name}}...",
    "availableVariables": ["first_name", "app_name", "app_genre"]
  }'
```

---

## Thompson Sampling

### How It Works

1. **Initialize Arms**: Each subject line variant starts with α=1, β=1 (uniform prior)
2. **Sample**: For each send, sample from Beta(α, β) for all arms
3. **Select**: Pick the arm with highest sampled value
4. **Observe**: Track opens/replies
5. **Update**: On success → α++, On failure → β++

### Example Convergence

```
Trial 1:   Arm A (α=1, β=1) samples 0.72
           Arm B (α=1, β=1) samples 0.45
           → Select A, user opens → A becomes (α=2, β=1)

Trial 100: Arm A (α=35, β=16) ≈ 68% success rate
           Arm B (α=12, β=39) ≈ 23% success rate
           → A wins 95%+ of selections (exploitation)
```

### Creating Arms

```bash
# Add subject line variants
curl -X POST https://admin.apexmediation.ee/api/autonomous/arms \
  -H "Content-Type: application/json" \
  -d '{
    "armType": "subject_line",
    "armName": "Direct Question",
    "armValue": "Quick question about {{app_name}} monetization"
  }'

curl -X POST https://admin.apexmediation.ee/api/autonomous/arms \
  -H "Content-Type: application/json" \
  -d '{
    "armType": "subject_line",
    "armName": "Revenue Focus",
    "armValue": "{{app_name}} - leaving ad revenue on the table?"
  }'
```

### Recording Feedback

```bash
# User opened email
curl -X POST https://admin.apexmediation.ee/api/autonomous/feedback \
  -H "Content-Type: application/json" \
  -d '{"armId": "uuid", "eventType": "open"}'

# User replied
curl -X POST https://admin.apexmediation.ee/api/autonomous/feedback \
  -H "Content-Type: application/json" \
  -d '{"armId": "uuid", "eventType": "reply"}'
```

---

## Lead Scoring

### Signal Extraction

The Analyst module extracts these signals from public data:

```typescript
// From app-ads.txt
- ads_txt_present: Boolean (file exists)
- ad_networks: ["google.com", "facebook.com", "applovin.com"]
- network_count: Number of ad networks

// From app store listing
- app_genre: "Games", "Entertainment", etc.
- update_date: Last app update
- rating_count: Number of reviews

// From website (if available)
- has_website: Boolean
- has_contact: Email discoverable
```

### Tier Thresholds

```sql
tier = CASE 
  WHEN overall_score >= 70 THEN 'whale'    -- High value, prioritize
  WHEN overall_score >= 40 THEN 'qualified' -- Worth pursuing
  ELSE 'cold'                               -- Low priority
END
```

### Example Scoring

```
App: "Puzzle Quest" by GameCo

Signals:
- app-ads.txt: YES (google.com, unity.com, applovin.com) → 100
- Network count: 3 → 60  
- Premium networks: Google, Unity → 70
- Genre: Games → 100
- Recent update: Yes → 80
- Has email: Yes → 100
- Has website: Yes → 100

Weighted Score: 
  (100×0.25) + (60×0.15) + (70×0.15) + (100×0.10) + (80×0.10) + (100×0.10) + (100×0.05)
  = 25 + 9 + 10.5 + 10 + 8 + 10 + 5
  = 77.5

Tier: WHALE ✅
```

---

## Email Classification

### Classification Flow

```
Email Arrives
     │
     ▼
┌─────────────────┐
│  Rule-Based     │ ← Fast path (regex patterns)
│  Classification │
└────────┬────────┘
         │
    Match Found?
    ├── YES → Return category (confidence: 95%)
    │
    └── NO
         │
         ▼
┌─────────────────┐
│  Naive Bayes    │ ← ML classification
│  Classifier     │
└────────┬────────┘
         │
    Confidence > 70%?
    ├── YES → Return category
    │
    └── NO → Flag for human review
```

### Training the Classifier

The classifier improves through:

1. **Seed Data**: Pre-loaded examples in migration
2. **Human Reviews**: Corrections added as training data
3. **Feedback Loop**: Reviewed messages improve accuracy

```bash
# Submit human review
curl -X POST https://admin.apexmediation.ee/api/autonomous/inbox/review \
  -H "Content-Type: application/json" \
  -d '{
    "messageId": "uuid",
    "correctCategory": "INTERESTED"
  }'
```

---

## Operations Guide

### Daily Operations

```bash
# Check system health
curl https://admin.apexmediation.ee/api/autonomous/health

# View statistics
curl https://admin.apexmediation.ee/api/autonomous/stats

# Check pending reviews
curl "https://admin.apexmediation.ee/api/autonomous/inbox?status=pending_review"
```

### Running Discovery

```bash
# Create a source
curl -X POST https://admin.apexmediation.ee/api/autonomous/sources \
  -H "Content-Type: application/json" \
  -d '{
    "name": "US Games",
    "sourceType": "itunes",
    "searchQuery": "arcade game",
    "category": "Games",
    "region": "us"
  }'

# Run discovery
curl -X POST https://admin.apexmediation.ee/api/autonomous/discover \
  -H "Content-Type: application/json" \
  -d '{"sourceId": "uuid-from-above"}'
```

### Scoring Contacts

```bash
# Score batch of contacts
curl -X POST https://admin.apexmediation.ee/api/autonomous/score \
  -H "Content-Type: application/json" \
  -d '{"limit": 100}'
```

### Composing Messages

```bash
# Generate message for contact
curl -X POST https://admin.apexmediation.ee/api/autonomous/compose \
  -H "Content-Type: application/json" \
  -d '{
    "contactId": "uuid",
    "firstName": "John",
    "company": "GameCo",
    "appName": "Puzzle Quest",
    "appGenre": "Games"
  }'
```

### Viewing Logs

```bash
# SSH to app server
./scripts/ops/hetzner_ssh.sh app

# View control-plane logs
journalctl -u control-plane -f
```

---

## Troubleshooting

### Common Issues

#### "column does not exist" errors
The database schema may be out of sync. Run the migration:
```bash
./scripts/ops/hetzner_ssh.sh db -- 'sudo -u postgres psql -d adproject -f /path/to/049_autonomous_sales_v2.sql'
```

#### No templates returned
Templates are filtered by `active = true`. Check:
```sql
SELECT name, template_type, active FROM sa_spintax_templates;
```

#### Thompson Sampler not learning
Ensure feedback is being recorded:
```sql
SELECT arm_name, total_trials, alpha, beta FROM sa_bandit_arms;
```

#### Classification always "UNKNOWN"
The Naive Bayes needs training data:
```sql
SELECT label, COUNT(*) FROM sa_bayes_training GROUP BY label;
```

### Debug Queries

```sql
-- Check discovery sources
SELECT name, status, total_contacts_found, last_scraped_at 
FROM sa_sources ORDER BY created_at DESC;

-- Check contact scoring
SELECT c.company_name, h.overall_score, h.tier
FROM sa_contacts c
JOIN sa_heuristic_scores h ON h.contact_id = c.id
ORDER BY h.overall_score DESC LIMIT 20;

-- Check template usage
SELECT name, times_used, open_rate, reply_rate
FROM sa_spintax_templates
ORDER BY times_used DESC;

-- Check bandit convergence
SELECT arm_name, total_trials, empirical_rate, 
       alpha::float / (alpha + beta) as expected_rate
FROM sa_bandit_arms
WHERE arm_type = 'subject_line'
ORDER BY empirical_rate DESC;

-- Check classification accuracy
SELECT classification, 
       COUNT(*) as total,
       AVG(classification_confidence) as avg_confidence
FROM sa_inbox_messages
GROUP BY classification;
```

---

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 2.0.0 | Jan 2026 | Initial release - Hunter, Analyst, Spintax, Thompson, Clerk |

---

## Related Documentation

- [Sales Automation Summary](SALES_AUTOMATION_SUMMARY.md) - V1 Cialdini system
- [Sales Automation Operations](SALES_AUTOMATION_OPERATIONS.md) - V1 operations
- [Cialdini Sales Strategy](CIALDINI_SALES_STRATEGY.md) - Psychology-based approach
