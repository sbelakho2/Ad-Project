# ApexMediation Infrastructure

Infrastructure as Code (IaC) and deployment configurations for the ApexMediation platform.

## 🏗️ Overview

The platform is designed for ultra-lean operations, targeting low monthly costs while maintaining high availability and global performance.

## 📋 Declarative Configuration

All deployment configuration is centralized in `deployment.toml`. This file is the **single source of truth** for:
- Server inventory
- Service definitions
- Port mappings
- Firewall rules
- Backup schedules
- Feature flags

### Quick Deploy

```bash
# Deploy all services (idempotent)
./deploy.sh all

# Deploy specific component
./deploy.sh backend
./deploy.sh console
./deploy.sh control-plane

# Dry run (show what would be done)
./deploy.sh all --dry-run

# Verify all services
./deploy.sh verify
```

## 🏛️ Server Architecture (February 2026)

| Server | IP | Role | Services |
|--------|-----|------|----------|
| **App Server** | 46.62.252.96 | Production Host | Backend (PM2), Control-Plane, Caddy, PgBouncer |
| **DB Server** | 77.42.44.35 | Database Host | PostgreSQL (via App Server PgBouncer only) |
| **Test Server** | 157.90.126.205 | Sales Automation & ML | **sales-api (PM2:4001)**, edge-worker (3500), inbound-smtp (2525) |

### Sales Automation Architecture

> **⚠️ CRITICAL:** All **Autonomous Sales V2** (outbound prospecting, campaign execution) runs on the **Test Server**.
> The **App Server control-plane is strictly a proxy client** that forwards `/api/autonomous/*` requests to Test Server via SSH tunnel.

```
┌─────────────────────────────────────────────────────────────────────┐
│                      APP SERVER (46.62.252.96)                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Control-Plane (Port 4000)                                   │    │
│  │   autonomous-api-proxy.ts → forwards to localhost:4001      │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ sales-api-tunnel.service (systemd)                          │    │
│  │   localhost:4001 ──SSH──> Test Server:4001                  │    │
│  └─────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
                              │ SSH Tunnel
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    TEST SERVER (157.90.126.205)                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ sales-api (PM2, Port 4001)                                  │    │
│  │   - Autonomous Orchestrator                                 │    │
│  │   - Spintax Engine                                          │    │
│  │   - Thompson Sampler                                        │    │
│  │   - Lead Scoring                                            │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ db-tunnel.service (systemd)                                 │    │
│  │   localhost:6432 ──SSH──> App Server:6432 ──> DB:5432       │    │
│  └─────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

## �🛠️ Stack

- **Cloud Providers**: Hetzner (Production).
- **Orchestration**: PM2 + systemd for product services; Docker Compose for monitoring and local development.
- **IaC**: Terraform 1.5+.
- **Database**: Managed PostgreSQL (DigitalOcean/Supabase), Upstash Redis.
- **Observability**: Prometheus, Grafana, Loki (Self-hosted stack).
- **Edge/CDN**: Cloudflare (DNS, WAF, R2 Storage).

## 📁 Directory Structure

```text
infrastructure/
├── docker/             # Custom Dockerfiles for services
├── terraform/          # IaC for cloud resources
├── helm/               # Legacy/optional Helm charts (not used for Hetzner production)
├── nginx/              # Reverse proxy and SSL configurations
├── production/         # Production-specific environment templates
└── scripts/            # Deployment and maintenance scripts
```

## 🚀 Deployment

### Local Production Simulation
To run the full stack locally in production mode:
```bash
docker-compose -f docker-compose.prod.yml up -d
```

### Production Checklist
Before deploying to production, ensure:
1.  **TLS** is enabled for all public endpoints.
2.  **DATABASE_URL** includes `sslmode=require`.
3.  **REDIS_URL** has a strong password configured.
4.  **CORS_ALLOWLIST** is restricted to authorized domains.

See `docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md` for more details.

## 📈 Monitoring

The infrastructure includes a self-hosted monitoring stack:
- **Grafana**: `https://grafana.apexmediation.ee`
- **Prometheus**: Internal scraping of `/metrics` from all services.
- **Alertmanager**: Wired to Slack and PagerDuty for critical incidents.
