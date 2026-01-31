# Hetzner Production Checklist

This is the production deployment runbook for ApexMediation on Hetzner infrastructure. It consolidates server setup, service deployment, database configuration, TLS termination, monitoring, and operational procedures.

> **Infrastructure Overview:**  
> - **App Server:** 46.62.252.96 (apexmediation.ee)  
> - **DB Server:** 77.42.44.35 (PostgreSQL only - **not directly accessible**, reach via App Server)  
> - **Test Server:** 157.90.126.205 (**ML workloads + SDK test harnesses**; managed and accessed same as app server)  
> - **Customer Domains:** apexmediation.ee, console.apexmediation.ee, api.apexmediation.ee  
> - **Operator Domain:** admin.apexmediation.ee (Control Plane - separate from Console)  
> - **SSH Access:** `ssh -i ~/.ssh/hetzner-db-mac root@<App Server IP>` (DB Server only reachable from App Server)
> - **Project Path:** `/opt/adproject` (NOT `/opt/apex` or `/var/www`)
> - **Deployment:** Git pull (preferred) or rsync
> - **Compose File:** `infrastructure/docker/docker-compose.hetzner.yml`

> **⚠️ CRITICAL: Production Paths**
>
> **All services run from `/opt/adproject`**. Do NOT deploy to `/var/www` or any other path.
>
> | Service | Production Path | Process Manager | Server |
> |---------|-----------------|-----------------|--------|
> | Backend | `/opt/adproject/backend` | PM2 | App |
> | Console | `/opt/adproject/console` | PM2 | App |
> | Website | `/opt/adproject/website` | PM2 | App |
> | Control Plane | `/opt/adproject/control-plane` | nohup/systemd | App |
> | **Sales API** | `/opt/adproject/sales-automation` | PM2 | **Test** |
> | Edge Worker | `/opt/adproject/control-plane` | systemd | **Test** |
>
> **Deployment commands MUST use `/opt/adproject` as the target.**

## Current Production Status (2026-01-25)

| Component | Status | Port | Server | Details |
|-----------|--------|------|--------|--------|
| Backend API | ✅ Running | 8080 | App | PM2 (Node) |
| Console Frontend | ✅ Running | 3000 | App | PM2 (Next.js standalone) |
| Website | ✅ Running | 3002 | App | Marketing site (Grafana uses 3001) |
| **Control Plane** | ✅ Running | 4000 | App | Refactored modular controllers |
| **Sales API** | ✅ Running | 4001 | **Test** | PM2 (Node) - **ALL sales automation runs here** |
| **Edge Worker** | ✅ Running | 3500 | **Test** | ML inference (ONNX 9-Dim, DistilBERT) |
| **ApexTunnel** | ✅ Running | 51820 | **All** | P2P Mesh Networking (Noise/Ed25519) |
| PostgreSQL 16 | ✅ Running | 5432 | DB | System service with BRIN/HLL extensions |
| Redis | ✅ Running | 6379 | App | System service, not Docker |
| Caddy | ✅ Running | 80/443 | App | Auto Let's Encrypt for all domains |
| Prometheus | ✅ Running | 9090 | App | Monitoring |
| Grafana | ✅ Running | 3001 | App | Dashboards |

### Sales Automation Architecture (CRITICAL)

> **⚠️ The Control Plane uses ApexTunnel for cross-server communication.**

> All sales automation work (orchestration, spintax, scoring, messaging) runs on the **Test Server**.
> ApexTunnel provides a low-latency, mutually authenticated P2P mesh between nodes.

| Component | Server | Port | Purpose |
|-----------|--------|------|---------|
| Control Plane | App | 4000 | Refactored controllers |
| **Sales API** | **Test** | 4001 | **ALL orchestration, spintax, Thompson sampler** |
| Edge Worker | Test | 3500 | ML inference (ONNX 9-Dim, classification) |
| DB Access | Test→DB | 5432 | Direct P2P connectivity via ApexTunnel |

**ApexTunnel Connectivity:**

| Service | Protocol | Identity | Purpose |
|---------|----------|----------|---------|
| `apextunnel.service` | Noise / UDP | Ed25519 | Secure P2P Mesh Overlay |

### Port Allocation (DO NOT CHANGE WITHOUT UPDATING CADDY)

| Hostname | Service | Local Port | Process |
|----------|---------|------------|---------|
| `admin.apexmediation.ee` | Control Plane (Admin) | 4000 | systemd |
| `api.apexmediation.ee` | Backend API | 8080 | PM2 |
| `console.apexmediation.ee` | Console (Customer) | 3000 | PM2 |
| `apexmediation.ee` / `www.apexmediation.ee` | Website | 3002 | PM2 |

> **Rule:** `admin.*` MUST route ONLY to port 4000. Never proxy admin UI paths to the customer console.

---

## 🚨 CRITICAL OPERATIONAL RULES - READ BEFORE ANY CHANGES

> **Incident History:** These rules exist because of real production incidents.
> Violating them has caused Control Plane outages and login failures.

### ❌ NEVER DO THESE

| Rule # | NEVER DO | Why It Breaks Things |
|--------|----------|---------------------|
| 1 | **Modify App Server `.env` files without explicit approval** | Control Plane + backend share config. One wrong edit crashes auth. |
| 2 | **Change `SALES_API_HOST` to the Test Server IP (157.90.126.205)** | ApexTunnel handles internal routing. Using external IP bypasses mesh. |
| 3 | **Deploy Sales API or Edge Worker to App Server** | These run ONLY on Test Server. |
| 4 | **Restart control-plane without checking for orphan processes** | `EADDRINUSE` on port 4000 will crash restart. |
| 5 | **Assume "fixing wiring" means changing hostnames** | ApexTunnel resolves internal nodes. |
| 6 | **Run sales automation orchestration jobs on App Server** | All execution happens on Test Server. |

### ✅ ALWAYS DO THESE

| Rule # | ALWAYS DO | How |
|--------|-----------|-----|
| 1 | **Verify ApexTunnel status before diagnosing connectivity** | `systemctl status apextunnel` |
| 2 | **Check for orphan processes before restarting services** | `lsof -ti :4000 \| xargs -r kill -9` |
| 3 | **Use Internal P2P IP for SALES_API_HOST on App Server** | ApexTunnel provides `10.0.0.x` addresses |
| 4 | **Test changes on Test Server FIRST** | Sales API + Edge Worker are safely isolated there |
| 5 | **Read existing `.env` files before making changes** | Understand current config |
| 6 | **Document any manual changes in this runbook** | Future operators need to know |

### The ApexTunnel Architecture (MEMORIZE THIS)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         APP SERVER (46.62.252.96)                       │
│                                                                         │
│  [Control Plane] (Port 4000)                                            │
│       │                                                                 │
│       ▼                                                                 │
│  [ApexTunnel] (P2P Mesh) ◄────────────────────────────────────────┐     │
└───────┬───────────────────────────────────────────────────────────│─────┘
        │                                                           │
        │ P2P (UDP/Noise)                                           │
        │                                                           │
┌───────▼───────────────────────────────────────────────────────────│─────┐
│                        TEST SERVER (157.90.126.205)               │     │
│                                                                   │     │
│  [ApexTunnel] ◄───────────────────────────────────────────┐       │     │
│       │                                                   │       │     │
│       ▼                                                   │       │     │
│  [Sales API] (Port 4001)                                  │       │     │
│  [Edge Worker] (Port 3500)                                │       │     │
└───────────────────────────────────────────────────────────│───────┘     │
                                                            │             │
                                                            │             │
┌───────────────────────────────────────────────────────────▼─────────────┐
│                        DB SERVER (77.42.44.35)                          │
│                                                                         │
│  [ApexTunnel] ◄─────────────────────────────────────────────────────────┘
│       │
│       ▼
│  [PostgreSQL] (Port 5432)
└─────────────────────────────────────────────────────────────────────────┘
```

### Emergency Recovery Commands

**Control Plane won't start (EADDRINUSE):**
```bash
ssh root@46.62.252.96 'lsof -ti :4000 | xargs -r kill -9 && systemctl start control-plane'
```

**Sales automation shows "unhealthy" after config change:**
```bash
# First, check what SALES_API_HOST is set to
ssh root@46.62.252.96 'grep SALES_API_HOST /opt/adproject/control-plane/.env'
# If it's NOT 127.0.0.1, fix it:
ssh root@46.62.252.96 'sed -i "s/SALES_API_HOST=.*/SALES_API_HOST=127.0.0.1/" /opt/adproject/control-plane/.env && systemctl restart control-plane'
```

**Verify tunnels are running:**
```bash
ssh root@46.62.252.96 'systemctl status sales-api-tunnel edge-tunnel --no-pager'
```

**Restart tunnels if needed:**
```bash
ssh root@46.62.252.96 'systemctl restart sales-api-tunnel edge-tunnel'
```

**Check full health status:**
```bash
curl -s https://admin.apexmediation.ee/api/autonomous/health | jq .
```

---

### Console Security Status (2025-12-17)

| Feature | Status | Notes |
|---------|--------|-------|
| RBAC Enforcement | ✅ Active | Settings/Billing protected by layouts |
| 2FA Available | ✅ Active | Settings → Security & 2FA |
| Session Security | ✅ Active | httpOnly cookies, CSRF protection |
| Operator Separation | ✅ Active | Admin routes redirect to control plane |

### Admin Access

The following user is configured with `admin` role for production verification:

| Role | Email |
|------|-------|
| Admin | `sales@apexmediation.ee` |

> **Credentials:** Stored in the team password manager / secret store (do not hard-code passwords in this runbook).


> **⚠️ Important Notes:**
> - **PostgreSQL runs on DB Server (77.42.44.35)** as a system service - NOT on App Server, NOT in Docker
> - **Redis runs on App Server** as a system service - NOT in Docker
> - **PgBouncer runs on App Server** as a system service (port 6432)
> - If PgBouncer is enabled for backend connections, set `DATABASE_URL` to `...@127.0.0.1:6432/...` on the App Server; otherwise connect directly to `77.42.44.35:5432`.
> - Backend queries PostgreSQL remotely via `DATABASE_URL` pointing to DB Server
> - Monitoring stack runs in Docker; product services run via PM2/systemd
> - Use `infrastructure/docker/docker-compose.hetzner.yml` (and/or `/opt/adproject/docker-compose.hetzner.yml` on-server) for monitoring containers
> - Website runs on port **3002** (Grafana uses 3001)
> - Control Plane runs as **systemd service** on port **4000**
> - **admin.apexmediation.ee → Control Plane (port 4000)** - completely separate from Console
> - **console.apexmediation.ee → Console (port 3000)** - customer-facing dashboard

### Environment File Precedence (CRITICAL)

- **Canonical env file for production:** `/opt/adproject/.env`
- **Backend runtime:** should receive `DATABASE_URL` via **PM2 env** (preferred). `/opt/adproject/.env` is a fallback for one-off scripts.
- **One-off scripts/CLIs (analytics rollups, etc.):** run from `/opt/adproject/backend` but should still resolve `DATABASE_URL` from `/opt/adproject/.env`.
- If `/opt/adproject/backend/.env` exists, treat it as **legacy** — do not rely on it for production DB settings.
- PostgreSQL on the DB server requires TLS; ensure connections are encrypted (backend/scripts enable SSL automatically for non-local DB hosts).

Safe verification (prints only host/port/db, never credentials):

- Use the repo helper `tools/print_db_url_targets.py` (VS Code task: `tmp: prod print DATABASE_URL targets (safe)`)

> **Operator Access:** The Control Plane at `https://admin.apexmediation.ee` provides operator telemetry and actions.
> Access requires IP allowlisting and separate operator authentication.
> See [Control Plane Architecture](../../Architecture/CONTROL_PLANE.md) for details.

---

## Pipeline Sanity Checks (Analytics + Billing)

Use these whenever “data looks stale” or after deploying ingestion/billing changes.

### A) Synthetic traffic sources (test server)

In addition to the long-running harness services, there is a high-signal end-to-end canary that:

- writes events via SDK ingestion (`X-SDK-Key`) and
- reads back via authenticated analytics API (JWT `TOKEN` or publisher `API_KEY=sk_*`).

Run it on the test server:

```bash
./scripts/ops/hetzner_ssh.sh test -- "set -euo pipefail; chmod +x /opt/adproject/scripts/ops/e2e_canary_verify.sh; bash /opt/adproject/scripts/ops/e2e_canary_verify.sh"
```

Notes:

- Readback uses a cache-busting query param to avoid false failures from Redis-cached analytics endpoints.
- The canary uses UTC day boundaries; around 00:00Z, “today” totals can reset.
- If `REPLICA_DATABASE_URL` is enabled, replication lag can delay read visibility after ingestion.

```bash
# Confirm synthetic harness services are running
./scripts/ops/hetzner_ssh.sh test -- "set -euo pipefail; \
  systemctl is-active web-sdk-harness.service ios-tv-api-harness.service slo-continuous.service; \
  echo; \
  systemctl status web-sdk-harness.service --no-pager -n 8; \
  echo; \
  systemctl status ios-tv-api-harness.service --no-pager -n 8; \
  echo; \
  systemctl status slo-continuous.service --no-pager -n 8"
```

### B) Analytics ingestion (DB facts + stage drain)

```bash
# Facts should have a fresh max(observed_at), and stage should be ~0 (draining)
./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; sudo -u postgres psql -d adproject -At -c \
\"SELECT 'impressions_fact' AS k, count(*) AS n, max(observed_at) AS max_ts
  FROM analytics_impressions
  WHERE observed_at > now() - interval '30 minutes'
UNION ALL
SELECT 'impressions_stage' AS k, count(*) AS n, max(observed_at) AS max_ts
  FROM analytics_impressions_stage
  WHERE observed_at > now() - interval '30 minutes'
UNION ALL
SELECT 'clicks_fact' AS k, count(*) AS n, max(observed_at) AS max_ts
  FROM analytics_clicks
  WHERE observed_at > now() - interval '30 minutes'
UNION ALL
SELECT 'clicks_stage' AS k, count(*) AS n, max(observed_at) AS max_ts
  FROM analytics_clicks_stage
  WHERE observed_at > now() - interval '30 minutes'
UNION ALL
SELECT 'revenue_events' AS k, count(*) AS n, max(observed_at) AS max_ts
  FROM analytics_revenue_events
  WHERE observed_at > now() - interval '30 minutes'
UNION ALL
SELECT 'revenue_stage' AS k, count(*) AS n, max(observed_at) AS max_ts
  FROM analytics_revenue_events_stage
  WHERE observed_at > now() - interval '30 minutes';\""
```

Optional attribution sanity check:

```bash
./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; sudo -u postgres psql -d adproject -At -c \
\"SELECT publisher_id, count(*) AS clicks_20m
  FROM analytics_clicks
  WHERE observed_at > now() - interval '20 minutes'
  GROUP BY publisher_id
  ORDER BY clicks_20m DESC;\""
```

Optional backend liveness/status probe (from app server):

```bash
./scripts/ops/hetzner_ssh.sh app -- "set -euo pipefail; \
  for path in /health /api/v1/status.json /api/v1/status; do \
    code=\$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080\$path || true); \
    echo \"\$path -> \$code\"; \
  done"
```

### C) Postgres safety: WAL archives + root disk headroom (CRITICAL)

Postgres can fail in surprising ways when the DB server root filesystem fills (e.g. failing to write internal init files, relation cache, temp files). In this stack the most common cause is WAL archiving growing on `/`.

Run on DB server (via jump host):

```bash
./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; \
  echo '[df]'; df -h / /mnt/HC_Volume_104191119 || true; \
  echo; \
  echo '[wal archive size]'; sudo du -sh /var/backups/postgresql/wal 2>/dev/null || true; \
  echo; \
  echo '[archiver health]'; sudo -u postgres psql -d adproject -P pager=off -c \"select * from pg_stat_archiver;\""
```

Expected:

- `/` has safe free space headroom (do not operate at ~100% used)
- WAL archive size is consistent with your PITR target window and prune jobs
- `pg_stat_archiver.failed_count` is not increasing

If `/var/backups/postgresql/wal` is large and living on `/`:

- Prefer relocating archived WAL off root (e.g. to the mounted volume) rather than just shortening retention.
- Ensure only one retention/prune job is active (multiple cron jobs can fight each other, or misconfigure the effective PITR window).

### D) Billing pipeline (tables + config smoke)

Billing tables may legitimately be empty until the first customer subscription/payment activity occurs.

Confirm billing routes exist (expect 401/403 without auth; 404 would indicate missing routes):

```bash
./scripts/ops/hetzner_ssh.sh app -- "set -euo pipefail; \
  for path in /api/v1/billing/usage/current /api/v1/billing/invoices /api/v1/billing/subscription; do \
    code=\$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080\$path || true); \
    echo \"\$path -> \$code\"; \
  done"
```

```bash
# Presence + latest timestamps (should be non-error; may still be empty)
./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; sudo -u postgres psql -d adproject -At -c \
\"SELECT 'invoices' AS t, count(*)::bigint AS n, max(created_at) AS max_created, max(updated_at) AS max_updated, max(paid_at) AS max_paid FROM invoices
UNION ALL
SELECT 'payment_reconciliations' AS t, count(*)::bigint AS n, max(created_at) AS max_created, max(resolved_at) AS max_updated, NULL::timestamptz AS max_paid FROM payment_reconciliations
UNION ALL
SELECT 'payment_failures' AS t, count(*)::bigint AS n, max(created_at) AS max_created, max(resolved_at) AS max_updated, max(next_retry_at) AS max_paid FROM payment_failures
UNION ALL
SELECT 'billing_audit_trail' AS t, count(*)::bigint AS n, max(created_at) AS max_created, max(timestamp) AS max_updated, NULL::timestamptz AS max_paid FROM billing_audit_trail
UNION ALL
SELECT 'billing_idempotency' AS t, count(*)::bigint AS n, max(created_at) AS max_created, NULL::timestamptz AS max_updated, NULL::timestamptz AS max_paid FROM billing_idempotency;\""
```

Optional Stripe connectivity check (read-only, no secrets printed):

```bash
./scripts/ops/hetzner_ssh.sh app -- "set -euo pipefail; cd /opt/adproject/backend; \
  node -e \"require('dotenv').config({ path: '/opt/adproject/.env' }); \
    const key=process.env.STRIPE_SECRET_KEY||''; \
    const mode=key.startsWith('sk_live_')?'live':(key.startsWith('sk_test_')?'test':(key.length?'unknown':'missing')); \
    console.log('stripe_key_mode='+mode); \
    if (key.length===0) process.exit(0); \
    const Stripe=require('stripe'); \
    (async()=>{ const stripe=new Stripe(key,{apiVersion:'2023-10-16'}); const acct=await stripe.accounts.retrieve(); console.log('stripe_ok=true'); console.log('stripe_account_id='+acct.id); })().catch(e=>{ console.log('stripe_ok=false'); console.log('stripe_error='+(e&&e.message?e.message:String(e))); process.exitCode=2; });\""
```

If you suspect billing is misconfigured, check backend logs for Stripe config errors (no secrets printed):

```bash
./scripts/ops/hetzner_ssh.sh app -- "set -euo pipefail; (pm2 logs backend --lines 2000 --nostream --raw 2>/dev/null || true) | \
  grep -F 'Stripe secret key not configured' | tail -n 20 || true"
```

---

## 1. Infrastructure Architecture

### 1.1 Three-Server Setup

```
┌─────────────────────────────────────┐     ┌─────────────────────────────────────┐
│         App Server                  │     │         DB Server                   │
│         46.62.252.96                │     │         77.42.44.35                 │
│                                     │     │         (NOT directly accessible)   │
│  ┌─────────────────────────────┐   │     │                                     │
│  │  Caddy (HTTPS termination)  │   │     │  ┌─────────────────────────────┐   │
│  │  - apexmediation.ee    :3002│   │     │  │  PostgreSQL 16              │   │
│  │  - console.*           :3000│   │     │  │  - Database: adproject      │   │
│  │  - api.*               :8080│   │     │  │  - 148+ tables              │   │
│  │  - admin.*             :4000│   │     │  │  - System service           │   │
│  └─────────────────────────────┘   │     │  │  - Data storage ONLY        │   │
│              │                      │     │  └─────────────────────────────┘   │
│  ┌───────────┴───────────┐         │     │                ▲                    │
│  │                       │         │     └────────────────┼────────────────────┘
│  ▼                       ▼         │                      │
│  ┌─────────┐     ┌─────────────┐   │                      │ TCP 5432
│  │ Console │     │   Backend   │───┼──────────────────────┘ (queries only)
│  │ :3000   │     │   :8080     │   │
│  └─────────┘     └─────────────┘   │
│                        │           │
│  ┌─────────┐     ┌─────────┐       │
│  │ Website │     │  Redis  │       │
│  │ :3002   │     │  :6379  │       │
│  │  (PM2)  │     │(systemd)│       │
│  └─────────┘     └─────────┘       │
│                                    │
│  ┌─────────────────────────────┐  │      ┌─────────────────────────────────────┐
│  │  Control Plane :4000        │  │      │         Test Server                 │
│  │  (systemd - operator only)  │  │      │         157.90.126.205              │
│  └──────────────┬──────────────┘  │      │         (ML + SDK Harnesses)        │
│                 │                  │      │                                     │
│  ┌──────────────┴──────────────┐  │      │  ┌─────────────────────────────┐   │
│  │  Edge Tunnel (systemd)      │  │      │  │  Edge Worker :3500          │   │
│  │  localhost:3500 ────────────┼──┼─────►│  │  - LaMini-Flan-T5-248M      │   │
│  │  (SSH tunnel to Test)       │  │      │  │  - DistilBERT classifier    │   │
│  └─────────────────────────────┘  │      │  │  - ~900MB RAM               │   │
│                                    │      │  └─────────────────────────────┘   │
│  ┌─────────────────────────────┐  │      │                                     │
│  │  Monitoring                 │  │      │  ┌─────────────────────────────┐   │
│  │  - Prometheus :9090         │  │      │  │  SDK Test Harnesses         │   │
│  │  - Grafana :3001            │  │      │  │  - web-sdk-harness          │   │
│  └─────────────────────────────┘  │      │  │  - ios-tv-api-harness       │   │
└────────────────────────────────────┘      │  │  - slo-continuous           │   │
                                            │  └─────────────────────────────┘   │
                                            └─────────────────────────────────────┘
```

### 1.2 Database Architecture (CRITICAL)

> **⚠️ PostgreSQL runs ONLY on the DB Server (77.42.44.35)**
>
> - The App Server does NOT run PostgreSQL
> - Backend connects to DB Server via `DATABASE_URL=postgresql://...@77.42.44.35:5432/adproject`
> - All data is stored on DB Server, not App Server
> - DB Server is NOT directly accessible via SSH from the internet (by design)
> - To access DB Server, first SSH to App Server, then SSH to DB Server

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     DATA FLOW (READ-ONLY from App Server perspective)   │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  App Server (46.62.252.96)              DB Server (77.42.44.35)        │
│  ┌─────────────────────────┐           ┌─────────────────────────┐     │
│  │ Backend Container       │           │ PostgreSQL 16           │     │
│  │                         │  SELECT   │                         │     │
│  │  - Queries data    ────────────────►│  - Stores ALL data      │     │
│  │  - CRUD operations ────────────────►│  - Runs migrations      │     │
│  │  - NO local DB     ◄────────────────│  - Backups              │     │
│  │                         │  RESULTS  │                         │     │
│  └─────────────────────────┘           └─────────────────────────┘     │
│                                                                         │
│  ❌ App Server does NOT have:          ✅ DB Server has:               │
│     - PostgreSQL container                - PostgreSQL 16 (systemd)    │
│     - Local database files                - /var/lib/postgresql/       │
│     - pg_dump/psql locally                - Backup scripts             │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 1.3 Network & Security

- **SSH Access:** Key-based only (`~/.ssh/hetzner-db-mac`)
- **DB Server SSH:** Only accessible FROM App Server (jump host pattern)
- **Firewall:** UFW enabled on App Server, allow 22, 80, 443
- **Database:** PostgreSQL on DB Server accepts connections only from App Server IP
- **Redis:** Runs on App Server as system service, password-protected

---

## 2. Server Setup

### 2.1 App Server (46.62.252.96)

Prerequisites:
- [x] Ubuntu 24.04 LTS
- [x] Non-root user disabled (using root with SSH key)
- [x] Docker & Docker Compose installed
- [x] UFW configured: 22, 80, 443

Services running:
- [x] Caddy (reverse proxy with auto TLS)
- [x] Backend API (Node.js on port 8080)
- [x] Console (Next.js on port 3000)
- [x] Redis (port 6379)
- [x] Prometheus (port 9090)
- [x] Grafana (port 3001)

### 2.2 DB Server (77.42.44.35)

Prerequisites:
- [x] Ubuntu 24.04 LTS
- [x] PostgreSQL 16 installed as system service
- [x] Firewall: Allow PostgreSQL from App Server only

Database:
- [x] Database name: `adproject`
- [x] 139+ tables migrated
- [x] Backups configured

### 2.3 Test Server (157.90.126.205)

> **Purpose:** Dedicated to **ML workloads**, **SDK test harnesses**, and **Sales Automation execution**. Isolates ML inference and autonomous sales from the production App Server to prevent memory/CPU contention.

> **⚠️ CRITICAL (January 2026):** This server now runs **ALL sales automation work**. The App Server control-plane is strictly a proxy client.

Prerequisites:
- [x] Ubuntu 24.04 LTS
- [x] Node.js 20+ installed
- [x] SSH key access (same key as App Server)
- [x] Direct SSH access (not via jump host)
- [x] PostgreSQL access via db-tunnel.service

Services running:
- [x] **Sales API** (autonomous sales execution on port 4001)
  - Autonomous Orchestrator (campaign execution)
  - Spintax Engine (email personalization)
  - Thompson Sampler (variant optimization)
  - Lead Scoring & Intent Classification
  - `sales-api` PM2 process (~600MB RAM)
- [x] **Edge Worker** (ML inference on port 3500)
  - LaMini-Flan-T5-248M (icebreaker generation)
  - DistilBERT (intent classification)
  - ~900MB RAM usage
- [x] **Inbound SMTP** (reply processing on port 2525)
  - `inbound-smtp.service` - Catches reply emails
- [x] **SDK Test Harnesses** (synthetic traffic)
  - `web-sdk-harness.service` - Web SDK events
  - `ios-tv-api-harness.service` - iOS/TV API events  
  - `slo-continuous.service` - SLO monitoring
- [x] **DB Tunnel** (database access)
  - `db-tunnel.service` - SSH tunnel to App Server PgBouncer (localhost:6432 → App:6432)

Connection to App Server:
- **Sales API Tunnel** (`sales-api-tunnel.service`) on App Server creates SSH tunnel
  - App Server localhost:4001 → Test Server:4001
- **Edge Tunnel** (`edge-tunnel.service`) on App Server creates SSH tunnel
  - App Server localhost:3500 → Test Server:3500
- Control Plane (App Server) proxies all `/api/autonomous/*` requests via tunnel
- Tunnels automatically reconnect if connection drops

Test Server Operations (run from local):
```bash
# Check sales-api status
./scripts/ops/hetzner_ssh.sh test -- "pm2 status"

# Check all systemd services
./scripts/ops/hetzner_ssh.sh test -- "sudo systemctl status db-tunnel.service inbound-smtp.service edge-worker.service"

# View sales-api logs
./scripts/ops/hetzner_ssh.sh test -- "pm2 logs sales-api --lines 50"

# Restart sales-api
./scripts/ops/hetzner_ssh.sh test -- "pm2 restart sales-api"

# Verify DB tunnel is working
./scripts/ops/hetzner_ssh.sh test -- "psql 'postgres://adproject:***@localhost:6432/adproject?sslmode=disable' -c 'SELECT 1'"
```

---

## 3. SSH Access

### 3.1 Using the Helper Script

```bash
# SSH to app server (default)
./scripts/ops/hetzner_ssh.sh

# SSH to app server explicitly
./scripts/ops/hetzner_ssh.sh app

# SSH to database server
./scripts/ops/hetzner_ssh.sh db

# SSH to test server
./scripts/ops/hetzner_ssh.sh test

# Run command on app server
./scripts/ops/hetzner_ssh.sh app -- "docker ps"

# Run command on database server
./scripts/ops/hetzner_ssh.sh db -- "sudo -u postgres psql -d adproject -c 'SELECT COUNT(*) FROM users;'"

# Run command on test server
./scripts/ops/hetzner_ssh.sh test -- "hostname && uptime"
```

### 3.2 Ops Scripts (Recommended)

These scripts live under `scripts/ops/` in this repo and are designed to be idempotent and production-safe.

- `scripts/ops/hetzner_ssh.sh`
  - SSH wrapper for Hetzner app/db/test servers.
  - DB access always goes through the app server (ProxyJump) by design.
- `scripts/ops/deploy-edge-worker.sh`
  - Deploys Edge Worker to **Test Server only** (157.90.126.205).
  - **Refuses to run on App Server** (server validation safeguard).
  - Rebuilds and restarts edge-worker.service.
- `scripts/ops/deploy-control-plane-sales.sh`
  - Deploys Control Plane to **App Server only** (46.62.252.96).
  - **Refuses to run on Test Server** (server validation safeguard).
  - Rebuilds and restarts control-plane.service.
- `scripts/ops/deploy-sales-automation.sh`
  - Combined deployment: Control Plane to App, Edge Worker to Test.
  - Orchestrates both deployments with proper validation.
- `scripts/ops/verify-deployment-architecture.sh`
  - Verifies services are running on correct servers.
  - Checks Edge Worker on Test, Control Plane on App.
- `scripts/ops/hetzner_enable_pitr.sh`
  - Ensures PITR-quality backups on the DB server:
    - Daily physical base backups under `/var/backups/postgresql/base/`
    - Enforces archived WAL retention to **72 hours (3 days)** — weekend-safe for teams without 24/7 on-call
    - WAL growth minimized (UNLOGGED high-churn staging tables + `wal_compression=lz4`)
    - Also reconciles common legacy WAL-prune cron jobs so they do not shorten the PITR window; always verify effective retention.
- `scripts/ops/prune_s3_wal.sh`
  - Prunes S3 WAL archives older than retention period (default: **7 days**)
  - Cron: `/etc/cron.d/adproject-s3-wal-prune` (daily at 5am)
  - Prevents unbounded S3 storage growth (Hetzner Object Storage pricing increases after 1 TB)
- `scripts/ops/hetzner_measure_wal_rate.sh`
  - Measures the *true* current WAL generation rate via `pg_stat_wal` delta and shows recent archive churn.
- `scripts/ops/hetzner_enable_transparency.sh`
  - Enables Transparency API and ensures required tables exist (team + transparency) on production.
  - Restarts backend safely (PM2 if present; docker-compose fallback).

### 3.3 Direct SSH

```bash
# App server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Database server (via app server jump host)
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35

# Test server (direct - same key, no jump host needed)
ssh -i ~/.ssh/hetzner-db-mac root@157.90.126.205
```

---

## 4. Service Management

### 4.1 PM2 Services (App Server)

The product services (backend/console/website) run under PM2 on the app server.

```bash
pm2 ls

pm2 logs backend --lines 200
pm2 logs console --lines 200
pm2 logs website --lines 200

pm2 restart backend
pm2 restart console
pm2 restart website
```

### 4.2 Docker Services (App Server)

Docker is primarily used for the monitoring stack (Grafana/Prometheus/Alertmanager/exporters).

```bash
# View running containers
docker ps

# View logs
docker logs grafana -f --tail 100
docker logs prometheus -f --tail 100

# Restart monitoring services
docker compose -f /opt/adproject/infrastructure/docker/docker-compose.hetzner.yml restart

# Full stack restart
docker compose -f /opt/adproject/infrastructure/docker/docker-compose.hetzner.yml down && \
  docker compose -f /opt/adproject/infrastructure/docker/docker-compose.hetzner.yml up -d
```

### 4.3 PostgreSQL (DB Server)

```bash
# Check status
systemctl status postgresql

# Connect to database
sudo -u postgres psql -d adproject

# Common queries
sudo -u postgres psql -d adproject -c "SELECT COUNT(*) FROM users;"
sudo -u postgres psql -d adproject -c "SELECT name, enabled FROM adapter_configs ORDER BY name;"
```

### 4.4 Redis (App Server)

```bash
# Check status
systemctl status redis-server

# Connect (with password)
redis-cli -a <password>

# Check memory
redis-cli -a <password> INFO memory
```

### 4.5 Edge Worker & Tunnel (Test Server ↔ App Server)

The Edge Worker runs on Test Server for ML inference. App Server connects via SSH tunnel.

**On App Server (Edge Tunnel):**
```bash
# Check tunnel status
systemctl status edge-tunnel

# View tunnel logs
journalctl -u edge-tunnel -n 50 --no-pager

# Restart tunnel
systemctl restart edge-tunnel

# Test tunnel connectivity
curl -s http://localhost:3500/health | python3 -m json.tool
```

**On Test Server (Edge Worker):**
```bash
# Check worker status
./scripts/ops/hetzner_ssh.sh test -- "systemctl status edge-worker --no-pager"

# View worker logs
./scripts/ops/hetzner_ssh.sh test -- "journalctl -u edge-worker -n 50 --no-pager"

# Restart worker
./scripts/ops/hetzner_ssh.sh test -- "systemctl restart edge-worker"

# Check health directly
./scripts/ops/hetzner_ssh.sh test -- "curl -s http://localhost:3500/health | python3 -m json.tool"
```

**Health Check Response (healthy state):**
```json
{
  "status": "healthy",
  "models": {
    "icebreaker": { "loaded": true, "modelName": "Xenova/LaMini-Flan-T5-248M" },
    "classifier": { "nbTrained": true, "deepModelLoaded": true }
  }
}
```

### 4.6 SDK Test Harnesses (Test Server)

```bash
# Check all harness services
./scripts/ops/hetzner_ssh.sh test -- "systemctl status web-sdk-harness ios-tv-api-harness slo-continuous --no-pager"

# View harness logs
./scripts/ops/hetzner_ssh.sh test -- "journalctl -u web-sdk-harness -n 20 --no-pager"
./scripts/ops/hetzner_ssh.sh test -- "journalctl -u ios-tv-api-harness -n 20 --no-pager"
./scripts/ops/hetzner_ssh.sh test -- "journalctl -u slo-continuous -n 20 --no-pager"
```

---

## 5. TLS & Domains

### 5.1 Caddy Configuration

Caddy handles automatic TLS certificate provisioning and renewal via Let's Encrypt.

#### Domain Routing (CRITICAL)

| Domain | Service | Port | Purpose |
|--------|---------|------|--------|
| `apexmediation.ee` | Website | 3002 | Marketing site (PM2) |
| `www.apexmediation.ee` | Website | 3002 | Marketing site (PM2) |
| `console.apexmediation.ee` | Console | 3000 | Customer dashboard |
| `api.apexmediation.ee` | Backend | 8080 | API server |
| `admin.apexmediation.ee` | Control Plane | 4000 | **Operator-only** (systemd) |

> **⚠️ CRITICAL: admin.apexmediation.ee and console.apexmediation.ee are SEPARATE services.**
> - `admin.*` → Control Plane (port 4000) - internal operator tools
> - `console.*` → Console (port 3000) - customer-facing dashboard
> - These must NEVER be combined in Caddy config!

#### 5.1.1 Production Caddyfile (`/etc/caddy/Caddyfile`)

```
# Marketing Website
apexmediation.ee, www.apexmediation.ee {
  reverse_proxy localhost:3002
}

# Customer Console (customer-facing dashboard)
console.apexmediation.ee {
  reverse_proxy localhost:3000
}

# Control Plane (operator-only - SEPARATE from Console)
admin.apexmediation.ee {
  reverse_proxy localhost:4000
}

# Backend API
api.apexmediation.ee {
  reverse_proxy localhost:8080
}
```

#### 5.1.2 Verify Caddy Configuration

```bash
# Reload Caddy config
caddy reload --config /etc/caddy/Caddyfile

# Verify routes
curl -I https://apexmediation.ee          # → 200 (Website)
curl -I https://console.apexmediation.ee  # → 200 (Console login)
curl -I https://admin.apexmediation.ee    # → 200 (Control Plane login)
curl -I https://api.apexmediation.ee/health # → 200 (Backend)
```

### 5.2 Certificate Status

```bash
# View Caddy certificates
caddy list-certs

# Reload Caddy config
caddy reload
```

---

## 6. Monitoring

### 6.1 Access Dashboards

- **Prometheus:** http://46.62.252.96:9090 (internal)
- **Grafana:** http://46.62.252.96:3001

### 6.2 Health Checks

```bash
# Backend health
curl https://api.apexmediation.ee/health

# Backend readiness
curl https://api.apexmediation.ee/ready

# Public / fail-open endpoints (used by marketing website)
curl -i https://api.apexmediation.ee/api/v1/status.json
curl -i https://api.apexmediation.ee/api/v1/slo.json
curl -i https://api.apexmediation.ee/api/v1/transparency/keys

# SDK parse-failure ingestion (Invalid Manifest Rate signal)
# Expect: HTTP 202 and a JSON body with success=true
curl -i -X POST https://api.apexmediation.ee/api/v1/analytics/events/manifest-parse \
  -H 'Content-Type: application/json' \
  -d '{"events":[{"protocol":"hls","sdk_platform":"runbook","error_code":"MANIFEST_PARSE_ERROR","error_message":"smoke test"}]}'

# Console
curl -I https://console.apexmediation.ee

# Marketing website key routes
curl -I https://apexmediation.ee
curl -I https://apexmediation.ee/status
curl -I https://apexmediation.ee/slo

# /signin is a client-side redirect to the Console login page
curl -s https://apexmediation.ee/signin | grep -E 'console\.apexmediation\.ee/login' || true
```

### 6.3 Prometheus Targets

All targets should show UP status at `/targets`:
- Backend API
- Console
- PostgreSQL exporter
- Redis exporter
- Node exporter
- Caddy

---

## 7. Database Operations

### 7.1 Running Migrations

```bash
# Production policy: no interactive/manual psql migrations.
#
# - Routine DB operations are automated (backups, WAL cleanup, health, disk checks).
# - Rollup-table rollout is automated by the backend at startup when analytics rollups are enabled.
# - For exceptional one-off schema changes, use a scripted, explicit migration runner ("--only")
#   rather than executing SQL manually.

# Rollup-table automation relies on the compiled backend containing:
# - dist/scripts/ensureRollupTables.js
# - dist/scripts/runMigrationsV2.js
# - dist/migrations/postgres/*.sql
# Verify on the App Server after deployment:
./scripts/ops/hetzner_ssh.sh app -- "set -euo pipefail; cd /opt/adproject/backend; ls -la dist/scripts/ensureRollupTables.js dist/scripts/runMigrationsV2.js dist/migrations/postgres | head"

# Analytics ingestion drift check (read-only):
#
# These relations must exist for production ingestion to persist events:
# - analytics_impressions / analytics_clicks / analytics_revenue_events
# - analytics_impressions_stage / analytics_clicks_stage / analytics_revenue_events_stage
#
# Safe check (read-only):
./scripts/ops/hetzner_ssh.sh db -- "sudo -u postgres psql -d adproject -P pager=off -At -c \"\
  SELECT 'analytics_impressions=' || COALESCE(to_regclass('analytics_impressions')::text,'MISSING');\
  SELECT 'analytics_clicks=' || COALESCE(to_regclass('analytics_clicks')::text,'MISSING');\
  SELECT 'analytics_revenue_events=' || COALESCE(to_regclass('analytics_revenue_events')::text,'MISSING');\
  SELECT 'analytics_impressions_stage=' || COALESCE(to_regclass('analytics_impressions_stage')::text,'MISSING');\
  SELECT 'analytics_clicks_stage=' || COALESCE(to_regclass('analytics_clicks_stage')::text,'MISSING');\
  SELECT 'analytics_revenue_events_stage=' || COALESCE(to_regclass('analytics_revenue_events_stage')::text,'MISSING');\
\""

# If drift is detected, apply the explicit migration from the backend repo:
./scripts/ops/hetzner_ssh.sh app -- "set -euo pipefail; cd /opt/adproject/backend; npm ci --silent; npm run migrate -- --only 20260105_000100_analytics_drift_repair"

# (Optional verification) Check backend logs for:
# - "Analytics rollups: ensuring rollup tables exist"
```

### 7.1.1 Analytics Rollups (Long-Running CLIs)

Long rollup runs can look like they “stall” when executed over SSH (no output for a while, or the SSH session drops). To make rollups reliable and to prevent duplicate concurrent runs, use the repo helper which:

- runs the job **detached** (background) and writes a server-side log file
- uses a singleton lock (`flock`) so only one rollup run happens at a time
- records the current log path so you can `tail` and `status` without remembering it

Run (from your local repo checkout):

```bash
# Start a rollup run in the background (recommended)
./scripts/ops/prod_analytics_rollups.sh start --start 2026-01-08 --end 2026-01-09

# Follow the current log
./scripts/ops/prod_analytics_rollups.sh tail

# Show whether a run is active + last log lines
./scripts/ops/prod_analytics_rollups.sh status
```

If you explicitly want to run in the foreground (less reliable for long ranges):

```bash
./scripts/ops/prod_analytics_rollups.sh start --start 2026-01-08 --end 2026-01-09 --fg
```

Stop a stuck run (rare; prefer letting it finish):

```bash
./scripts/ops/prod_analytics_rollups.sh stop
```

### 7.2 Backups

PostgreSQL backups and PITR hygiene are configured on the DB server via cron.

There are two layers of backup automation in production:

1) **Legacy logical dumps** (pg_dump custom format)
- Script: `/usr/local/bin/pg_backup_apex.sh`
- Cron: `/etc/cron.d/apex-pg-backup` (daily)
- Output:
  - `/var/backups/postgresql/daily/*.dump`
  - `/var/backups/postgresql/weekly/*.dump` (Sundays)

2) **PITR (physical base backups + WAL archives)**
- Ensured by running `scripts/ops/hetzner_enable_pitr.sh` from this repo (idempotent).
- Output:
  - Base backups: `/var/backups/postgresql/base/base_YYYY-MM-DD.tar.gz` (daily)
  - Archived WAL (local): `/mnt/HC_Volume_104191119/postgresql-backups/wal/` (retention: **72 hours**)
  - Archived WAL (S3): `s3://wal/wal/` at `hel1.your-objectstorage.com` (retention: **7 days**)

  ### 7.2.1 S3/Object Storage WAL Archive (Offsite)

  If offsite WAL archiving is enabled, the archive is stored in **S3-compatible Object Storage**.

  **Current endpoint (Hetzner Object Storage):** `https://hel1.your-objectstorage.com`

  **Bucket:** `wal`

  **Purpose:** offsite WAL redundancy to complement on-server PITR retention.

  **Read-only verification (no secrets printed):**

  ```bash
  # Check bucket exists (requires credentials on DB server)
  ./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; \
    AWS_ACCESS_KEY_ID=*** AWS_SECRET_ACCESS_KEY=*** AWS_DEFAULT_REGION=us-east-1 \
    aws --endpoint-url https://hel1.your-objectstorage.com s3api head-bucket --bucket wal"
  ```

  **Optional listing (safe metadata only):**

  ```bash
  ./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; \
    AWS_ACCESS_KEY_ID=*** AWS_SECRET_ACCESS_KEY=*** AWS_DEFAULT_REGION=us-east-1 \
    aws --endpoint-url https://hel1.your-objectstorage.com s3api list-objects-v2 --bucket wal --max-items 10"
  ```

  > **Note:** Credentials should be stored on the DB server only. Do not place credentials on App/Test servers.

  **S3 WAL Retention (CRITICAL for cost control):**

  S3 WAL is pruned by `/opt/adproject/ops/prune_s3_wal.sh` running daily via cron.

  | Tier | Retention | Storage Est. | Purpose |
  |------|-----------|--------------|---------|
  | Local | 72 hours | ~7.5 GB | Fast PITR for recent incidents |
  | S3 | 7 days | ~17.5 GB | Extended PITR / disaster recovery |

  > **Cost alert:** Hetzner Object Storage pricing increases significantly after **1 TB**.
  > At ~2.5 GB/day WAL, 7-day retention = ~17.5 GB (safe). Monitor with:
  > ```bash
  > ./scripts/ops/hetzner_ssh.sh db -- "source /opt/adproject/ops/s3_archive.env && \
  >   export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_DEFAULT_REGION && \
  >   aws s3 ls s3://wal/wal/ --endpoint-url \$S3_ENDPOINT --summarize | tail -3"
  > ```

> **Important:** Multiple cron jobs affect WAL retention. Current active crons:
> - `/etc/cron.d/adproject-wal-retention` — local 72h prune (hourly)
> - `/etc/cron.d/adproject-s3-wal-prune` — S3 7-day prune (daily 5am)
> - `/etc/cron.d/apex-postgres-backup-retention` — legacy, updated to match 72h
> Always verify the effective prune window after any changes.

Run/verify (recommended):

```bash
# Enforce PITR-quality setup (idempotent)
./scripts/ops/hetzner_enable_pitr.sh

# Optional: relocate WAL archive off root filesystem
# WAL_ARCHIVE_DIR=/mnt/HC_Volume_104191119/postgresql/wal ./scripts/ops/hetzner_enable_pitr.sh

# Verify (no secrets)
./scripts/ops/hetzner_ssh.sh db -- "du -sh /var/backups/postgresql/base /var/backups/postgresql/wal /var/backups/postgresql/daily /var/backups/postgresql/weekly || true"
./scripts/ops/hetzner_ssh.sh db -- "sudo -u postgres psql -d adproject -P pager=off -At -c \"select pg_size_pretty(pg_database_size('adproject'));\""
./scripts/ops/hetzner_measure_wal_rate.sh 300
```

PITR restore procedure: [docs/PITR_RECOVERY.md](../../PITR_RECOVERY.md)

#### PITR Drill / Verification Checklist (Recommended)

Goal: ensure PITR is *actually* possible for >=24h (effective retention), and that the restore procedure is still valid.

1) Verify backups exist (on DB server):

```bash
./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; \
  echo '== base backups =='; \
  ls -1t /var/backups/postgresql/base/base_*.tar.gz 2>/dev/null | head -n 5 || true; \
  echo; \
  echo '== WAL archive size =='; \
  du -sh /var/backups/postgresql/wal 2>/dev/null || true"
```

2) Verify the *effective* WAL retention window (watch out for multiple cron jobs):

```bash
./scripts/ops/hetzner_ssh.sh db -- "set -euo pipefail; \
  echo '== cron (adproject) =='; \
  ls -la /etc/cron.d/adproject-pitr /etc/cron.d/adproject-wal-retention 2>/dev/null || true; \
  echo; \
  echo '== cron (legacy, if present) =='; \
  ls -la /etc/cron.d/apex-postgres-backup-retention 2>/dev/null || true; \
  echo; \
  echo '== WAL oldest/newest timestamps =='; \
  if [[ -d /var/backups/postgresql/wal ]]; then \
    find /var/backups/postgresql/wal -type f -name '0000*' -printf '%TY-%Tm-%TdT%TH:%TM:%TSZ %p\\n' 2>/dev/null | sort | { head -n 1; tail -n 1; } || true; \
  fi"
```

3) Sanity-check current WAL generation rate (authoritative):

```bash
./scripts/ops/hetzner_measure_wal_rate.sh 300
```

4) Perform a restore drill in a *non-production* environment using the documented procedure:

- Follow: [docs/PITR_RECOVERY.md](../../PITR_RECOVERY.md)
- Record the drill outcome (RPO/RTO, any deviations) in the ops evidence folder.

### 7.3 Table Counts

```bash
./scripts/ops/hetzner_ssh.sh db -- "sudo -u postgres psql -d adproject -c \"
  SELECT schemaname, COUNT(*) as tables
  FROM pg_tables
  WHERE schemaname = 'public'
  GROUP BY schemaname;
\""
```

---

## 8. Deployment Procedure

> **Deployment Methods (in order of preference):**
> 1. **Git Pull** - Fastest, simplest (requires deploy key setup)
> 2. **Rsync** - Good for pre-built artifacts
> 3. **Tarball** - Fallback method
>
> Note: Docker compose on the app server is primarily used for the monitoring stack.
> The product services (backend/console/website) run via PM2/systemd.

### 8.0 Git-Based Deployment (Recommended)

Git is initialized at `/opt/adproject`. Once you add the server's deploy key to GitHub, deployments become simple:

**One-time Setup: Add Deploy Key to GitHub**

1. Copy this public key:
   ```
   ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJRHgjXYF6tIU72sZVY9LREzLXWX2You1Bpq6m5oGOy3 root@ApexMain
   ```
2. Go to GitHub → Repository → Settings → Deploy Keys → Add deploy key
3. Paste the key, name it "Hetzner Production", check "Allow write access" (optional)

**Deploy via Git:**

```bash
# From local machine - commit and push
cd /path/to/Ad-Project
git add -A && git commit -m "feat: your changes" && git push origin main

# SSH to server and pull
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

cd /opt/adproject
git pull origin main

# Rebuild and restart affected services
cd backend && npm run build && pm2 restart backend
# OR for console: cd console && npm run build && pm2 restart console
# OR for website: cd website && npm run build && pm2 restart website
```

**Quick One-Liner (after deploy key is set up):**

```bash
# Deploy backend
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 'cd /opt/adproject && git pull && cd backend && npm run build && pm2 restart backend'

# Deploy all services
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 'cd /opt/adproject && git pull && \
  cd backend && npm run build && pm2 restart backend && \
  cd ../website && npm run build && pm2 restart website'
```

### 8.1 Quick Reference - Working Commands

```bash
# From your local machine:

# 1. Create deployment tarball
tar -czvf /tmp/deploy.tar.gz --exclude='node_modules' --exclude='.next' --exclude='dist' \
  backend/src backend/package.json \
  console/src console/package.json \
  website/src website/public \
  website/package.json website/package-lock.json \
  website/next.config.js website/tsconfig.json \
  website/tailwind.config.ts website/postcss.config.js \
  control-plane/src control-plane/package.json

# 2. Upload to production
scp -i ~/.ssh/hetzner-db-mac /tmp/deploy.tar.gz root@46.62.252.96:/opt/adproject/

# 3. Deploy (SSH in and run)
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

cd /opt/adproject
tar -xzvf deploy.tar.gz

cd /opt/adproject/backend
npm ci
npm run build
pm2 restart backend

cd /opt/adproject/console
npm ci
npm run build
pm2 restart console

cd /opt/adproject/website
npm ci
npm run build
pm2 start npm --name website -- start -- -p 3002
pm2 save

# 4. Verify
curl http://localhost:8080/health
curl -I http://localhost:3000
curl -I http://localhost:3002
```

### 8.2 Deploy Backend

```bash
# From local machine
cd /path/to/Ad-Project

# Create backend-only tarball
tar -czvf /tmp/backend-deploy.tar.gz --exclude='node_modules' \
  backend/src backend/package.json backend/tsconfig.json

# Upload
scp -i ~/.ssh/hetzner-db-mac /tmp/backend-deploy.tar.gz root@46.62.252.96:/opt/adproject/

# SSH and deploy
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd /opt/adproject
tar -xzvf backend-deploy.tar.gz
cd /opt/adproject/backend
npm ci
npm run build
pm2 restart backend

# Verify health
curl http://localhost:8080/health
```

### 8.3 Deploy Console

```bash
# From local machine
tar -czvf /tmp/console-deploy.tar.gz --exclude='node_modules' --exclude='.next' \
  console/src console/package.json console/next.config.js console/tsconfig.json

scp -i ~/.ssh/hetzner-db-mac /tmp/console-deploy.tar.gz root@46.62.252.96:/opt/adproject/

ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd /opt/adproject
tar -xzvf console-deploy.tar.gz
cd /opt/adproject/console
npm ci
npm run build
pm2 restart console

curl -I http://localhost:3000
```

### 8.4 Deploy Website

> **⚠️ CRITICAL:** The PM2 process runs from `/opt/adproject/website`. Do NOT deploy to `/var/www/website` or any other path.

Website runs on **port 3002** (Grafana uses 3001). There are two deployment methods:

#### 8.4.1 Rsync Method (Preferred - Pre-built)

Build locally and rsync pre-built artifacts. Faster and more reliable.

> **⚠️ CRITICAL: Standalone Static Assets**  
> Next.js standalone builds do NOT include static assets automatically.  
> You MUST copy `.next/static` and `public` into `.next/standalone/` after rsync.

```bash
# From local machine (website directory)
cd /path/to/Ad-Project/website

# 1. Build locally
npm run build

# 2. Rsync build artifacts directly to production
#    IMPORTANT: Target must be /opt/adproject/website NOT /var/www/website
rsync -avz --delete \
  -e "ssh -i ~/.ssh/hetzner-db-mac" \
  .next package.json node_modules public next.config.js \
  root@46.62.252.96:/opt/adproject/website/

# 3. Copy static assets into standalone folder (REQUIRED for CSS/JS to work)
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "\
  cp -r /opt/adproject/website/.next/static /opt/adproject/website/.next/standalone/.next/ && \
  cp -r /opt/adproject/website/public /opt/adproject/website/.next/standalone/"

# 4. Restart PM2 to pick up changes
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 restart website --update-env"

# 5. Verify deployment
curl -I https://apexmediation.ee
curl -I https://apexmediation.ee/_next/static/css/  # Should return 200, not 308/404
```

#### 8.4.2 Tarball Method (Alternative - Server-built)

Upload source and build on server. Use when rsync is not practical.

```bash
# From local machine (repo root)
tar -czvf /tmp/website-deploy.tar.gz \
  --exclude='node_modules' --exclude='.next' --exclude='test-results' \
  website/src website/public \
  website/package.json website/package-lock.json \
  website/next.config.js website/tsconfig.json \
  website/tailwind.config.ts website/postcss.config.js

scp -i ~/.ssh/hetzner-db-mac /tmp/website-deploy.tar.gz root@46.62.252.96:/opt/adproject/

ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd /opt/adproject
tar -xzvf website-deploy.tar.gz

cd /opt/adproject/website
npm ci
npm run build

# Start/restart via PM2 (keeps it pinned on port 3002)
pm2 start npm --name website -- start -- -p 3002
pm2 save

curl -I http://localhost:3002
```

#### 8.4.3 Common Mistakes to Avoid

| ❌ Wrong | ✅ Correct |
|----------|------------|
| Deploy to `/var/www/website` | Deploy to `/opt/adproject/website` |
| Deploy to `/opt/apex` | Deploy to `/opt/adproject` |
| rsync to wrong path then "restart" | Verify PM2 cwd first: `pm2 show website` |
| Forget to restart PM2 | Always run `pm2 restart website` |

**Verify PM2 is running from correct directory:**
```bash
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 show website | grep cwd"
# Should output: cwd: /opt/adproject/website
```

### 8.5 Deploy Control Plane

```bash
# Control plane runs as systemd service, not Docker
# From local machine
tar -czvf /tmp/control-plane-deploy.tar.gz --exclude='node_modules' \
  control-plane/src control-plane/package.json control-plane/tsconfig.json control-plane/migrations

scp -i ~/.ssh/hetzner-db-mac /tmp/control-plane-deploy.tar.gz root@46.62.252.96:/opt/adproject/

ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd /opt/adproject
tar -xzvf control-plane-deploy.tar.gz
cd control-plane
npm install
npm run build
systemctl restart control-plane

# Verify
curl http://localhost:4000/health
systemctl status control-plane
```

### 8.6 Port Reference

| Service | Container Port | Host Port | Notes |
|---------|---------------|-----------|-------|
| Backend | 8080 | 8080 | API server |
| Console | 3000 | 3000 | Next.js standalone |
| Website | N/A | 3002 | PM2 process (Grafana uses 3001) |
| Control Plane | 4000 | 4000 | Systemd service (not Docker) |
| Redis | 6379 | 6379 | System service (not Docker) |
| PostgreSQL | 5432 | 5432 | DB server (not Docker) |
| Grafana | 3000 | 3001 | Monitoring |
| Prometheus | 9090 | 9090 | Monitoring |

### 8.7 Compose Files Reference

| File | Purpose | Use When |
|------|---------|----------|
| `docker-compose.yml` | Local development | Running locally with Docker Postgres/Redis |
| `docker-compose.hetzner.yml` | Production (legacy) | Present on-server; not used for product services in current setup |
| `docker-compose.dev.yml` | Dev overlay | Local development with volume mounts |

Monitoring stack (Grafana/Prometheus/exporters) uses:
- `infrastructure/docker/docker-compose.hetzner.yml`

---

## 9. Rollback Procedure

### 9.1 Quick Rollback

```bash
# Preferred: git-based rollback helper (requires git checkout on /opt/adproject)
./scripts/ops/hetzner_rollback.sh --service backend --ref <git-tag-or-sha>

# Fallback: tarball rollback (when git is unavailable)
# - Re-upload the older tarball and repeat the relevant deploy steps in Section 8.
# - Then restart only the affected service:
#   - backend/console/website: pm2 restart <svc>
#   - website: pm2 restart website

# Verify
curl https://api.apexmediation.ee/health
```

### 9.2 Database Rollback

```bash
# SSH to DB server (via app server)
./scripts/ops/hetzner_ssh.sh db

# Logical backups are in pg_dump custom format (.dump)
ls -la /var/backups/postgresql/daily/

# Physical base backups (for PITR) are in:
# /var/backups/postgresql/base/

# Recommended: restore into a new database first
sudo -u postgres createdb adproject_restore
sudo -u postgres pg_restore -d adproject_restore /var/backups/postgresql/daily/<backup_file>.dump
```

PITR restore procedure (when required): [docs/PITR_RECOVERY.md](../../PITR_RECOVERY.md)

---

## 10. Troubleshooting

### 10.1 Backend Not Responding

```bash
# Check container status
docker ps | grep backend

# Check logs
docker logs backend --tail 100

# Check database connectivity
docker exec backend npm run db:check

# Restart
docker compose restart backend
```

### 10.2 Database Connection Issues

```bash
# From app server, test connectivity
nc -zv 77.42.44.35 5432

# Check PostgreSQL is running (on DB server)
systemctl status postgresql

# Check PostgreSQL logs
tail -100 /var/log/postgresql/postgresql-16-main.log
```

### 10.3 Redis Issues

```bash
# Check Redis status
systemctl status redis

# Check Redis logs
journalctl -u redis --since "1 hour ago"

# Restart Redis
systemctl restart redis
```

### 10.4 TLS/Certificate Issues

```bash
# Check Caddy status
systemctl status caddy

# View Caddy logs
journalctl -u caddy --since "1 hour ago"

# Force certificate renewal
caddy reload
```

### 10.5 Website Deployment Issues

**Problem: Pages return 404 after deployment**

```bash
# 1. First, verify PM2 is running and check its working directory
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 show website"

# Look for the "cwd" line - it MUST be /opt/adproject/website
# If cwd shows a different path, that's the problem
```

**Problem: Deployed to wrong directory**

This is a common mistake. If you deployed to `/var/www/website` but PM2 runs from `/opt/adproject/website`:

```bash
# Fix: Re-deploy to the correct directory
# From local machine
cd /path/to/Ad-Project/website
npm run build

rsync -avz --delete \
  -e "ssh -i ~/.ssh/hetzner-db-mac" \
  .next package.json node_modules public next.config.js \
  root@46.62.252.96:/opt/adproject/website/

ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 restart website --update-env"
```

**Problem: PM2 process not running**

```bash
# Check PM2 status
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 list"

# If website is not listed or status is "errored", start fresh
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd /opt/adproject/website
pm2 delete website 2>/dev/null
pm2 start npm --name website -- start -- -p 3002
pm2 save
```

**Problem: Website builds but pages still 404**

```bash
# Check if .next directory contains the pages
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "ls -la /opt/adproject/website/.next/server/app"

# If empty or missing pages, rebuild on server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd /opt/adproject/website
npm run build
pm2 restart website
```

**Quick diagnostic checklist:**

| Check | Command | Expected |
|-------|---------|----------|
| PM2 running | `pm2 list` | website: online |
| Correct cwd | `pm2 show website \| grep cwd` | `/opt/adproject/website` |
| Port 3002 | `curl -I localhost:3002` | HTTP 200 |
| .next exists | `ls /opt/adproject/website/.next` | Files present |

---

## 11. Evidence & Validation

### 11.1 Running Validation Tests

```bash
# VRA usefulness test
./scripts/ops/vra_usefulness_test.sh

# VRA overnight stability test
./scripts/ops/vra_overnight_stability.sh

# Synthetic load test
./scripts/ops/vra_synthetic_load.sh
```

### 11.2 E2E Tests Against Production

```bash
# Run Playwright tests
cd console
npx playwright test --config=playwright.droplet.config.ts
```

---

## 12. Migration Notes

### 12.1 From DigitalOcean

The platform migrated from DigitalOcean (164.90.232.151) to Hetzner in December 2025.

Key differences:
| Aspect | DigitalOcean | Hetzner |
|--------|--------------|---------|
| Servers | 1 droplet | 2 servers (App + DB) |
| Database | DO Managed Postgres | Self-hosted PostgreSQL 16 |
| TLS | Nginx + Certbot | Caddy (auto Let's Encrypt) |
| SSH | `deploy@164.90.232.151` | `root@46.62.252.96` / `root@77.42.44.35` |

### 12.2 Deprecated Files

The following files have been removed or deprecated:
- `.github/workflows/deploy-do.yml` - Deleted
- `docs/Internal/Infrastructure/DO_READINESS_CHECKLIST.md` - Deleted
- `docs/Internal/Deployment/DO_INITIAL_BOOT_COMMANDS.md` - Deleted
- `infrastructure/database/do-managed-postgres-roles.sql` - Deleted
- `scripts/ops/do_ssh.sh` - Deprecated (use `hetzner_ssh.sh`)
- `backend/deploy-backend.sh` - Deprecated

---

## Related Documentation

- [HETZNER_POSTGRES_DESIGN.md](../../Architecture/HETZNER_POSTGRES_DESIGN.md) - Architecture overview
- [INFRASTRUCTURE_MIGRATION_PLAN.md](../Infrastructure/INFRASTRUCTURE_MIGRATION_PLAN.md) - Migration history
- [MONITORING_INFRASTRUCTURE.md](../../Monitoring/MONITORING_INFRASTRUCTURE.md) - Monitoring setup
- [ROLLBACK.md](../../runbooks/ROLLBACK.md) - Rollback procedures

### Security Documentation

- [Console/RBAC_SYSTEM.md](../Console/RBAC_SYSTEM.md) - Role-based access control
- [Console/SECURITY_CHECKLIST.md](../Console/SECURITY_CHECKLIST.md) - Security audit checklist
- [Console/SESSION_AUTH.md](../Console/SESSION_AUTH.md) - Session and CSRF configuration
- [Security/TWO_FACTOR_AUTH.md](../Security/TWO_FACTOR_AUTH.md) - 2FA implementation
- [Security/SECRETS_MANAGEMENT_SETUP.md](../Security/SECRETS_MANAGEMENT_SETUP.md) - Key management

---

## 13. Continuous SLO Data Population (Test Server 157.90.126.205)

### 13.1 Purpose

To keep the public SLO page at `https://apexmediation.ee/slo` populated with live, representative metrics, we run continuous synthetic activity from a dedicated test server. This protects production from artificial load while ensuring SLO/SLA transparency remains meaningful before and during early customer ramp-up.

Key points:
- Runs from a separate test server: 157.90.126.205
- Managed operationally the same way as the App Server (same SSH key/process conventions)
- Exercises: analytics ingestion, status/slo endpoints, and a minimal SSAI session path
- NO test harness code runs on the App Server. The App Server only runs the normal product services.
- Data is ingested and treated as REAL production data (tied to a designated dummy publisher and its apps)
- MUST be disabled once sufficient customer data is present (see 13.5)

### 13.2 Test Server Access

SSH (same key as App/DB servers):
```bash
ssh -i ~/.ssh/hetzner-db-mac root@157.90.126.205
```

Project layout on the test server should mirror `/opt/adproject` (rsync/tarball deploy acceptable).

### 13.3 Continuous Runner Script

An orchestrator script is provided:
```bash
./scripts/ops/slo_continuous_test.sh
```

Environment variables (store in `/opt/adproject/.env.slo` on the test server):
- `API_URL` (default: https://api.apexmediation.ee)
- `TOKEN` (optional) Bearer token (JWT) that belongs to the dummy publisher user (NOT an admin)
- `API_KEY` (optional, recommended) Publisher API key (`sk_*`) for programmatic access to analytics read APIs (more stable than JWT)
- `SDK_KEY` (required) SDK key owned by the dummy publisher (maps activity to the correct tenant)
- `PUBLISHER_ID` (required) Dummy publisher UUID
- App IDs for each SDK owned by the dummy publisher (required to exercise Console, billing, multi-app flows):
  - `APP_ID_ANDROID`, `APP_ID_IOS`, `APP_ID_WEB`, `APP_ID_UNITY`, `APP_ID_CTV`
- Optional tuning: `CYCLE_MINUTES` (default 30), `RATE_EPM` (events/min, default 20), `SLEEP_BETWEEN` (sec, default 60)

Template file is provided: `infrastructure/production/.env.slo.example`.

Example manual run on the test server:
```bash
cd /opt/adproject
# 1) Prepare env file with the dummy publisher and per-SDK apps
cp /opt/adproject/infrastructure/production/.env.slo.example /opt/adproject/.env.slo
vi /opt/adproject/.env.slo   # fill TOKEN, SDK_KEY, PUBLISHER_ID, APP_ID_*

# 2) Start the runner (uses /opt/adproject/.env.slo)
nohup ./scripts/ops/slo_continuous_test.sh > /var/log/slo_continuous_test.out 2>&1 &
disown
```

The runner performs cycles of:
- Warming public endpoints: `/api/v1/status.json`, `/api/v1/slo.json`
- Synthetic analytics load via `scripts/ops/vra_synthetic_load.sh`
- Minimal SSAI smoke: creates an SSAI session and attempts to fetch a manifest (non-fatal)

Important constraints (do not violate):
- The synthetic generator MUST run only on the Test Server. Do not run these scripts on the App Server.
- The generator MUST use the same public ingestion routes used by real SDKs: `/api/v1/analytics/events/*`.
- Events MUST be sent with `is_test_mode=false` so they are included in production analytics rollups and reporting.
- The App Server must keep analytics rollups enabled (default) so synthetic events appear in Console and power VRA inputs.

### 13.4 Systemd (Recommended) or Cron Setup

Systemd service (continuous) — do NOT place secrets directly in the unit; use EnvironmentFile:
```ini
# /etc/systemd/system/slo-continuous.service
[Unit]
Description=Apex Continuous SLO Data Generator
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/adproject
EnvironmentFile=/opt/adproject/.env.slo
ExecStart=/opt/adproject/scripts/ops/slo_continuous_test.sh
Restart=always
RestartSec=10
StandardOutput=append:/var/log/slo_continuous_test.log
StandardError=inherit

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
systemctl daemon-reload
systemctl enable --now slo-continuous.service
systemctl status slo-continuous.service
```

Cron alternative (hourly cycle as a stateless job):
```bash
crontab -e
# Run a 20-minute cycle every hour (the script will read /opt/adproject/.env.slo)
0 * * * * cd /opt/adproject && CYCLE_MINUTES=20 RATE_EPM=20 SLEEP_BETWEEN=0 ./scripts/ops/slo_continuous_test.sh >> /var/log/slo_continuous_test.log 2>&1
```

### 13.5 Turn Off Policy (Once Customer Data Is Available)

Constant live testing must be turned OFF once real customer traffic sufficiently populates SLO metrics.

Disable systemd service:
```bash
systemctl disable --now slo-continuous.service
```

Disable cron (if used):
```bash
crontab -e
# remove the slo_continuous_test entry
```

Retain the ability to run the script on-demand for diagnostics, but do not leave continuous generators running once production volumes are healthy.

### 13.6 SDK Test Apps & SSAI Coverage

To maximize data diversity, keep SDK test apps running where feasible and ensure SSAI flows are exercised:
- Android / Web / Unity / CTV (tvOS/Android TV) test apps: keep at least one instance per platform active (emulator or device). Where available, use any provided soak/run scripts under `Test Apps/` folders.
- SSAI: The orchestrator performs a minimal smoke by creating a session. For deeper SSAI coverage, schedule periodic session creation and manifest fetches using representative assets.

Prerequisites for the dummy publisher account (must be completed once):
- Dummy user exists and owns a dummy publisher/tenant (record the `PUBLISHER_ID`).
- Create five apps under this tenant and record their IDs: Android, iOS, Web, Unity, CTV.
- Ensure an `SDK_KEY` issued for this tenant is available and configured in `.env.slo`.

Monitoring and checks:
- Grafana SLO dashboards should show steady, non-zero activity during synthetic runs.
- Website SLO page (`/slo`) must render populated metrics; `/status` should show an UP state.
- Console (dummy publisher) should show non-zero data in dashboard/analytics, sourced from normal `/api/v1/revenue/*` APIs.

### 13.7 Operational Notes – Recording Test App Downtime

If any SDK test app instance is intentionally stopped or experiencing issues:
- Record the downtime here with time and platform.
- Update expected coverage so on-call engineers know which signals are synthetic vs. organic.

Template:
```
[YYYY-MM-DD HH:MMZ] Android TV test app offline for maintenance (expected 2h)
[YYYY-MM-DD HH:MMZ] iOS test app crash loop detected; investigating
```
