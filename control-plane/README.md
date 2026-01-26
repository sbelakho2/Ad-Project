# ApexMediation Control Plane

Internal management service for the ApexMediation platform. Handles administrative tasks, compliance reporting, and infrastructure orchestration.

> ## 🚨 CRITICAL: DO NOT MODIFY APP SERVER CONFIG WITHOUT EXPLICIT APPROVAL
> 
> **Incident History (January 2026):** Unauthorized changes to `/opt/adproject/control-plane/.env` on the App Server caused Control Plane crashes and login failures.
> 
> **Key Rules:**
> - `SALES_API_HOST` MUST be `127.0.0.1` (NOT the Test Server IP) - SSH tunnel handles forwarding
> - NEVER deploy Sales API or Edge Worker code to the App Server - they run ONLY on Test Server
> - Before restarting Control Plane, kill orphan processes: `lsof -ti :4000 | xargs -r kill -9`
> - The Control Plane is a **PROXY ONLY** for sales automation - all execution happens on Test Server
> 
> **Recovery:**
> ```bash
> # If you accidentally changed SALES_API_HOST to Test Server IP:
> ssh root@46.62.252.96 'sed -i "s/SALES_API_HOST=.*/SALES_API_HOST=127.0.0.1/" /opt/adproject/control-plane/.env && systemctl restart control-plane'
> ```
> 
> See [HETZNER_PRODUCTION_CHECKLIST.md](../docs/Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md) for full operational rules.

## 🏗️ Architecture

The Control Plane is a separate Node.js service with elevated privileges to perform sensitive operations:

- **Database Separation**: Uses three distinct connection pools (Read-only, Operator, Break-glass) to enforce the principle of least privilege.
- **Compliance Engine**: Generates Estonian VAT and Annual reports using real-time ledger data.
- **Operator Portal**: Internal API surface for device approvals, partition management, and system overrides.

## 🛠️ Tech Stack

- **Runtime**: Node.js 20+
- **Language**: TypeScript
- **Framework**: Express.js
- **Database**: PostgreSQL (multi-role access)
- **Email**: SMTP integration for device verification challenges.

## 🚀 Key Features

### Estonian Compliance
Automated generation of reports for Estonian e-government systems:
- **VAT Reports (KMD)**: Quarterly summaries of domestic, EU, and international sales.
- **KMD INF**: Automated attachment for transactions > 1000 EUR.
- **Annual Reports**: Full financial statements (Profit/Loss, Assets, Liabilities, Equity) derived from the ledger.
- **TSD Reports**: Payroll and tax declarations.
- **PDF Generation**: Professional, branded PDFs for all compliance documents.

### CEO & Business Analytics
Real-time monitoring of platform performance and financial health:
- **Core Metrics**: Impressions, Revenue, and Active Publisher tracking with WoW/DoD comparisons.
- **CEO Metrics**: Estimated Runway, Net Cash Flow (30d), Customer LTV, and Churn Rate.
- **Advanced Analytics**: Signup cohort retention analysis and linear growth revenue forecasting (6-month outlook).

### Accounting & Reconciliation
- **Bank Reconciliation**: Automated matching of bank statement entries (SEPA/Wise) against internal invoices and expenses.
- **Audit Logging**: Mandatory persistence of all operator actions to the `audit_logs` table with 7-year retention.
- **Impersonation**: Secure, JWT-based "one-click" impersonation for troubleshooting user issues.

### Infrastructure Management
- **Partition Control**: Automates PostgreSQL table partitioning for high-volume analytics data (ad_events, bid_logs, etc.).
- **Job Orchestration**: Real-time health monitoring of Autopilot background jobs and Migration Studio.
- **System Telemetry**: Unified view of App health, Database sessions, and Host metrics (disk/CPU/RAM).
- **Break-glass Access**: Highly audited emergency access for system-level remediation.

## 🚦 Getting Started

### Prerequisites
- PostgreSQL 16
- Access to the main platform database

### Installation
```bash
cd control-plane
npm install
```

### Development
```bash
npm run dev
```

### Compliance Smoke Test
Run the compliance smoke test using an operator JWT.

Required env:
- `CONTROL_PLANE_URL` (defaults to `http://localhost:4000` if unset)
- `OPERATOR_TOKEN` (operator JWT signed with `OPERATOR_JWT_SECRET`)

Example (local):
```bash
OPERATOR_TOKEN=$(node -e "const jwt=require('jsonwebtoken'); const secret=process.env.OPERATOR_JWT_SECRET; const email=(process.env.OPERATOR_CREDENTIALS||'').split(',')[0]?.split(':')[0]||'operator@apexmediation.ee'; console.log(jwt.sign({ sub: email, role: 'operator' }, secret, { expiresIn: '2h' }));") \
CONTROL_PLANE_URL=http://localhost:4000 \
node scripts/smoke-test-compliance.ts
```

## 🛡️ Security & Auditing

- **Audit Logging**: Every operator action is logged with actor ID, reason, and timestamp.
- **Device Approval**: New administrative devices require email-based verification challenges.
- **Role-Based Access**: Restricted to `apex_operator` and `apex_readonly` database users for standard operations.

## 🤖 Autonomous Sales System (V2.6)

> **⚠️ CRITICAL (January 2026):** The Control Plane is **STRICTLY A PROXY CLIENT** for sales automation.
> **ALL sales automation work** (orchestration, spintax, Thompson sampling, scoring) runs on the **Test Server** (157.90.126.205:4001).
> This Control Plane forwards `/api/autonomous/*` requests to the Test Server via SSH tunnel (`sales-api-tunnel.service`).

### Architecture

| Server | IP | Role |
|--------|-----|------|
| **App Server** (this Control Plane) | 46.62.252.96 | **Proxy client only** - forwards to Test Server |
| **Test Server** | 157.90.126.205:4001 | **Execution server** - runs ALL sales logic |

```
App Server (Control Plane)        Test Server (sales-api)
┌─────────────────────────────┐   ┌─────────────────────────────┐
│ /api/autonomous/* ──────────┼───│→ Orchestrator, Spintax,     │
│    (proxy router)           │   │   Thompson Sampler, Scoring │
│                             │   │                             │
│ sales-api-tunnel.service    │   │ PM2: sales-api (port 4001)  │
│    localhost:4001 ─SSH──────┼───│→ 4001                       │
└─────────────────────────────┘   └─────────────────────────────┘
```

Zero-cost, self-improving outbound sales automation:

### Features
- **Lead Discovery**: iTunes Search API scraping (no auth needed)
- **Lead Scoring**: Heuristic signals (app-ads.txt, ad networks, genre)
- **Competitor Targeting ("Sniper")**: Detects IronSource, Unity, AppLovin users for priority outreach
- **Thompson Sampling**: Multi-armed bandit A/B tests subject lines
- **Closed-Loop Learning**: Webhooks automatically update Sampler from email responses
- **Email Classification**: Naive Bayes + rules for response categorization

### Key Endpoints (proxied to Test Server)
```
GET  /api/autonomous/health           - Health check
GET  /api/autonomous/stats            - System statistics
POST /api/autonomous/discover         - Run discovery (max batch: 10)
POST /api/autonomous/score            - Score contacts (max batch: 100)
POST /api/autonomous/compose          - Generate personalized message
GET  /api/autonomous/competitor-leads - Get leads using competitor networks
POST /api/autonomous/webhook/email-events - Closed-loop email events
POST /api/autonomous/webhook/sendgrid     - SendGrid inbound parse
```

### Operations Commands
```bash
# Check Test Server sales-api status
./scripts/ops/hetzner_ssh.sh test -- "pm2 status"

# View sales-api logs
./scripts/ops/hetzner_ssh.sh test -- "pm2 logs sales-api --lines 50"

# Restart sales-api
./scripts/ops/hetzner_ssh.sh test -- "pm2 restart sales-api"

# Check SSH tunnel on App Server
./scripts/ops/hetzner_ssh.sh app -- "sudo systemctl status sales-api-tunnel.service"
```

### Documentation
- [API Reference](../docs/Developer-Facing/AUTONOMOUS_SALES_API.md)
- [Full Guide](../docs/Internal/Sales/AUTONOMOUS_SALES_V2.md)
- [Quick Reference](../docs/Internal/Sales/AUTONOMOUS_SALES_V2_QUICK_REF.md)
