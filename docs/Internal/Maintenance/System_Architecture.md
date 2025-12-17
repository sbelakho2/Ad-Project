# System Architecture & Maintenance Guide

**Last Updated:** December 17, 2025  
**Target Audience:** New Engineers, DevOps, System Administrators

---

## 1. System Overview

ApexMediation is a high-performance ad mediation platform designed for transparency and verifiable auction logs. The system is built as a modular monolith with supporting microservices, deployed on bare-metal infrastructure (Hetzner) for performance and cost efficiency.

### High-Level Architecture

```mermaid
graph TD
    User[User / SDK] -->|HTTPS| LB[Caddy Reverse Proxy]
    
    subgraph "App Server (Hetzner)"
        LB -->|:3000| Console[Console (Next.js)]
        LB -->|:3002| Website[Website (Next.js)]
        LB -->|:8080| Backend[Backend API (Node.js)]
        LB -->|:4000| ControlPlane[Control Plane (Node.js)]
        
        Backend -->|Read/Write| Redis[Redis Cache]
        Backend -->|gRPC/HTTP| ML[ML Services]
    end
    
    subgraph "DB Server (Hetzner)"
        Backend -->|Read/Write| DB[PostgreSQL 16]
        ControlPlane -->|Read Only| DB
    end
    
    subgraph "Monitoring"
        Prometheus -->|Scrape| Backend
        Prometheus -->|Scrape| Caddy
        Grafana -->|Query| Prometheus
    end
```

---

## 2. Infrastructure & Deployment

The production environment runs on Hetzner Cloud using a split-server architecture to isolate database load from application logic.

### 2.1 Server Inventory

| Role | IP Address | OS | Key Services |
|------|------------|----|--------------|
| **App Server** | `46.62.252.96` | Ubuntu | Caddy, Backend, Console, Website, Redis, Control Plane |
| **DB Server** | `77.42.44.35` | Ubuntu | PostgreSQL 16 |
| **Test Server** | `157.90.126.205` | Ubuntu | Synthetic Load Generators |

### 2.2 Directory Structure (Production)

All services are deployed to `/opt/adproject`. **Do not use `/var/www`**.

```
/opt/adproject/
├── backend/          # Dockerized Node.js API
├── console/          # Dockerized Next.js Admin Dashboard
├── website/          # PM2-managed Next.js Marketing Site
├── control-plane/    # Systemd-managed Internal Tooling
├── infrastructure/   # Docker Compose & Configs
└── monitoring/       # Prometheus & Alertmanager configs
```

### 2.3 Process Management

We use a mix of Docker, PM2, and Systemd based on the service requirements:

- **Docker**: Backend, Console (Ease of deployment, isolation).
- **PM2**: Website (Next.js standalone mode, easy updates).
- **Systemd**: PostgreSQL, Redis, Control Plane (Performance, stability, OS integration).

### 2.4 Networking & Security

- **Reverse Proxy**: Caddy handles all incoming traffic on ports 80/443. It automatically manages Let's Encrypt certificates.
- **Firewall**:
    - **App Server**: Open ports 80, 443, 22. Internal ports (8080, 3000, etc.) are blocked from outside.
    - **DB Server**: Accepts connections on 5432 **only** from App Server IP.
- **SSH Access**: Key-based auth only (`~/.ssh/hetzner-db-mac`). Root login enabled (for now).

---

## 3. Core Components

### 3.1 Backend (`/backend`)

The core logic of the platform.
- **Stack**: Node.js, TypeScript, Express.
- **Key Modules**:
    - `rtb/`: Real-Time Bidding engine. Handles auction logic.
    - `ssai/`: Server-Side Ad Insertion. Stitches ads into video streams.
    - `billing/`: Stripe integration, invoice generation.
    - `transparency/`: Cryptographic signing of auction logs.
    - `fraud/`: Integration with ML fraud detection.
- **Configuration**: Environment variables managed via `docker-compose.prod.yml` or `.env`.

### 3.2 Console (`/console`)

The publisher and advertiser facing dashboard.
- **Stack**: Next.js (App Router), Tailwind CSS.
- **Features**: Analytics dashboards, App management, Billing settings.
- **Auth**: JWT-based authentication with the Backend.

### 3.3 Website (`/website`)

The public marketing site and documentation.
- **Stack**: Next.js (App Router).
- **Deployment**: Built as a standalone app, run with PM2.
- **Features**: Dynamic SLO page, Documentation rendering.

### 3.4 Control Plane (`/control-plane`)

Internal tool for system operators.
- **Stack**: Node.js.
- **Purpose**: "Break-glass" actions, deep system inspection, manual overrides.
- **Security**: Protected by strict IP allowlist and separate credentials.

### 3.5 ML & Services (`/services`, `/ML`)

- **Inference**: Python-based services for fraud detection and bid optimization.
- **Autopilot**: Automated campaign optimization service.

---

## 4. Data Layer

### 4.1 PostgreSQL

- **Version**: 16
- **Role**: Primary source of truth.
- **Database**: `apexmediation`
- **Key Schemas**: `public` (core), `analytics` (timescale-like data), `audit` (logs).
- **Backups**: Automated daily backups (check `infrastructure/production/backup-scripts`).

### 4.2 Redis

- **Role**: Caching, Session Store, Job Queues (BullMQ).
- **Persistence**: RDB snapshots enabled.
- **Key Usage**:
    - RTB Caching (critical for low latency).
    - Rate Limiting counters.
    - User Sessions.

---

## 5. Maintenance & Operations

### 5.1 Deployment

Deployments are currently manual via `rsync` and restart commands. **No CI/CD pipeline pushes directly to prod yet.**

**Standard Deployment Flow:**
1.  Build locally (`npm run build`).
2.  Sync files to server (`rsync`).
3.  Restart service (`docker compose restart` or `pm2 restart`).

Refer to `docs/Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md` for exact commands.

### 5.2 Monitoring

- **Prometheus**: Scrapes metrics from Backend (`/metrics`), Caddy, and Node Exporters.
- **Grafana**: Visualizes metrics. Accessible at `https://console.apexmediation.ee/monitoring` (tunnel required or internal).
- **Logs**:
    - Docker logs: `docker compose logs -f backend`
    - PM2 logs: `pm2 logs website`
    - System logs: `journalctl -u postgresql`

### 5.3 Common Tasks

**Restart Backend:**
```bash
cd /opt/adproject/infrastructure
docker compose -f docker-compose.prod.yml restart backend
```

**Restart Website:**
```bash
pm2 restart website
```

**Check DB Connection:**
```bash
# From App Server
psql -h 77.42.44.35 -U apex_app -d apexmediation
```

---

## 6. Onboarding for Developers

### 6.1 Local Development Setup

1.  **Prerequisites**: Node.js 20+, Docker, Python 3.11+.
2.  **Install Dependencies**:
    ```bash
    npm install
    cd backend && npm install
    cd ../console && npm install
    ```
3.  **Start Infrastructure**:
    ```bash
    docker compose up -d postgres redis
    ```
4.  **Run Backend**:
    ```bash
    cd backend
    npm run dev
    ```
5.  **Run Console**:
    ```bash
    cd console
    npm run dev
    ```

### 6.2 Key Concepts to Understand

- **Auction Logs**: We cryptographically sign every auction. Understand `backend/src/services/transparency`.
- **SDK Keys**: We use per-app keys (`sk_live_...`). Understand `backend/src/middleware/appApiKeyAuth.ts`.
- **SSAI**: We stitch ads on the server. Understand `backend/src/services/ssai`.

---

## 7. Troubleshooting

- **"502 Bad Gateway"**: Usually means the Backend or Console container is down. Check `docker ps` and logs.
- **"Database Connection Error"**: Check if DB server is reachable (`ping 77.42.44.35`) and Postgres is running.
- **"SLO Page Loading Forever"**: Check `/api/public/slo` endpoint response. If null, it means no data in window.

---

**End of Document**
