# Hetzner Production Checklist

This is the production deployment runbook for ApexMediation on Hetzner infrastructure. It consolidates server setup, service deployment, database configuration, TLS termination, monitoring, and operational procedures.

> **Infrastructure Overview:**  
> - **App Server:** 46.62.252.96 (apexmediation.ee)  
> - **DB Server:** 77.42.44.35  
> - **Domains:** apexmediation.ee, console.apexmediation.ee, api.apexmediation.ee  
> - **SSH Access:** `ssh -i ~/.ssh/hetzner-db-mac root@<IP>`
> - **Project Path:** `/opt/adproject` (NOT `/opt/apex` or `/var/www`)
> - **Deployment:** Tarball upload or rsync (no git on server)
> - **Compose File:** `docker-compose.hetzner.yml` (not `docker-compose.yml`)

> **⚠️ CRITICAL: Production Paths**
>
> **All services run from `/opt/adproject`**. Do NOT deploy to `/var/www` or any other path.
>
> | Service | Production Path | Process Manager |
> |---------|-----------------|-----------------|
> | Backend | `/opt/adproject/backend` | Docker |
> | Console | `/opt/adproject/console` | Docker |
> | Website | `/opt/adproject/website` | PM2 |
> | Control Plane | `/opt/adproject/control-plane` | systemd |
>
> **Deployment commands MUST use `/opt/adproject` as the target.**

## Current Production Status (2025-12-17)

| Component | Status | Port | Details |
|-----------|--------|------|---------|
| Backend API | ✅ Running | 8080 | PostgreSQL/Redis/Queues UP |
| Console Frontend | ✅ Running | 3000 | Next.js standalone |
| Website | ✅ Running | 3002 | Marketing site (Grafana uses 3001) |
| **Control Plane** | ✅ Running | 4000 | Systemd service, IP-restricted |
| PostgreSQL 16 | ✅ Running | 5432 | DB Server (system service, not Docker) |
| Redis | ✅ Running | 6379 | App Server (system service, not Docker) |
| Caddy | ✅ Running | 80/443 | Auto Let's Encrypt for all domains |
| Prometheus | ✅ Running | 9090 | Monitoring |
| Grafana | ✅ Running | 3001 | Dashboards |

### Console Security Status (2025-12-17)

| Feature | Status | Notes |
|---------|--------|-------|
| RBAC Enforcement | ✅ Active | Settings/Billing protected by layouts |
| 2FA Available | ✅ Active | Settings → Security & 2FA |
| Session Security | ✅ Active | httpOnly cookies, CSRF protection |
| Operator Separation | ✅ Active | Admin routes redirect to control plane |

> **⚠️ Important Notes:**
> - Redis and PostgreSQL run as **system services**, NOT Docker containers
> - Use `docker-compose.hetzner.yml` to avoid port conflicts
> - Website runs on port **3002** (Grafana uses 3001)
> - Control Plane runs as **systemd service**, not Docker

> **Operator Access:** The Control Plane at `https://admin.apexmediation.ee` provides operator telemetry and actions.
> Access requires IP allowlisting and separate operator authentication.
> See [Control Plane Architecture](../../Architecture/CONTROL_PLANE.md) for details.

---

## 1. Infrastructure Architecture

### 1.1 Two-Server Setup

```
┌─────────────────────────────────────┐     ┌─────────────────────────────────────┐
│         App Server                  │     │         DB Server                   │
│         46.62.252.96                │     │         77.42.44.35                 │
│                                     │     │                                     │
│  ┌─────────────────────────────┐   │     │  ┌─────────────────────────────┐   │
│  │  Caddy (HTTPS termination)  │   │     │  │  PostgreSQL 16              │   │
│  │  - apexmediation.ee         │   │     │  │  - Database: adproject      │   │
│  │  - console.apexmediation.ee │   │     │  │  - 139+ tables              │   │
│  │  - api.apexmediation.ee     │   │     │  │  - System service           │   │
│  └─────────────────────────────┘   │     │  └─────────────────────────────┘   │
│              │                      │     │                                     │
│  ┌───────────┴───────────┐         │     └─────────────────────────────────────┘
│  │                       │         │                    ▲
│  ▼                       ▼         │                    │
│  ┌─────────┐     ┌─────────────┐   │                    │
│  │ Console │     │   Backend   │───┼────────────────────┘
│  │ :3000   │     │   :8080     │   │
│  └─────────┘     └─────────────┘   │
│                        │           │
│                        ▼           │
│                  ┌─────────┐       │
│                  │  Redis  │       │
│                  │  :6379  │       │
│                  └─────────┘       │
│                                    │
│  ┌─────────────────────────────┐  │
│  │  Monitoring                 │  │
│  │  - Prometheus :9090         │  │
│  │  - Grafana :3001            │  │
│  └─────────────────────────────┘  │
└────────────────────────────────────┘
```

### 1.2 Network & Security

- **SSH Access:** Key-based only (`~/.ssh/hetzner-db-mac`)
- **Firewall:** UFW enabled, allow 22, 80, 443
- **Database:** PostgreSQL only accessible from App Server IP
- **Redis:** Password authentication, localhost only

---

## 2. Server Setup

### 2.1 App Server (46.62.252.96)

Prerequisites:
- [x] Ubuntu 22.04 LTS
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
- [x] Ubuntu 22.04 LTS
- [x] PostgreSQL 16 installed as system service
- [x] Firewall: Allow PostgreSQL from App Server only

Database:
- [x] Database name: `adproject`
- [x] 139+ tables migrated
- [x] Backups configured

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

# Run command on app server
./scripts/ops/hetzner_ssh.sh app -- "docker ps"

# Run command on database server
./scripts/ops/hetzner_ssh.sh db -- "sudo -u postgres psql -d adproject -c 'SELECT COUNT(*) FROM users;'"
```

### 3.2 Direct SSH

```bash
# App server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Database server
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35
```

---

## 4. Service Management

### 4.1 Docker Services (App Server)

```bash
# View running containers
docker ps

# View logs
docker logs backend -f --tail 100
docker logs console -f --tail 100

# Restart services
docker compose restart backend
docker compose restart console

# Full stack restart
docker compose down && docker compose up -d
```

### 4.2 PostgreSQL (DB Server)

```bash
# Check status
systemctl status postgresql

# Connect to database
sudo -u postgres psql -d adproject

# Common queries
sudo -u postgres psql -d adproject -c "SELECT COUNT(*) FROM users;"
sudo -u postgres psql -d adproject -c "SELECT name, enabled FROM adapter_configs ORDER BY name;"
```

### 4.3 Redis (App Server)

```bash
# Check status
systemctl status redis

# Connect (with password)
redis-cli -a <password>

# Check memory
redis-cli -a <password> INFO memory
```

---

## 5. TLS & Domains

### 5.1 Caddy Configuration

Caddy handles automatic TLS certificate provisioning and renewal via Let's Encrypt.

Domains served:
- `apexmediation.ee` / `www.apexmediation.ee` → Website (port 3002)
- `console.apexmediation.ee` → Console (port 3000)
- `api.apexmediation.ee` → Backend API (port 8080)

Operator access:
- `admin.apexmediation.ee` → Control Plane (port 4000)

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
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Run migrations
cd /opt/adproject && docker compose -f docker-compose.hetzner.yml exec backend npm run migrate
```

### 7.2 Backups

PostgreSQL backups are configured on the DB server:

```bash
# Manual backup
sudo -u postgres pg_dump adproject > /var/backups/postgres/adproject_$(date +%Y%m%d_%H%M%S).sql

# List backups
ls -la /var/backups/postgres/
```

### 7.3 Table Counts

```bash
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35 "sudo -u postgres psql -d adproject -c \"
  SELECT schemaname, COUNT(*) as tables
  FROM pg_tables
  WHERE schemaname = 'public'
  GROUP BY schemaname;
\""
```

---

## 8. Deployment Procedure

> **IMPORTANT:** The production server does NOT have git access. Deployment is done via tarball upload.
> Use `docker-compose.hetzner.yml` (not `docker-compose.yml`) as it excludes Postgres/Redis containers
> (they run as system services).

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
docker compose -f docker-compose.hetzner.yml build backend console
docker compose -f docker-compose.hetzner.yml up -d backend console

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
docker compose -f docker-compose.hetzner.yml build backend
docker compose -f docker-compose.hetzner.yml up -d backend

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
docker compose -f docker-compose.hetzner.yml build console
docker compose -f docker-compose.hetzner.yml up -d console

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
curl -I https://apexmediation.ee/_next/static/css/  # Should return 200, not 404
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
| `docker-compose.hetzner.yml` | Production | Hetzner server (excludes Postgres/Redis) |
| `docker-compose.dev.yml` | Dev overlay | Local development with volume mounts |

---

## 9. Rollback Procedure

### 9.1 Quick Rollback

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Navigate to project
cd /opt/adproject

# Rollback uses a previously known-good tarball (production server has no git).
# Re-upload the older tarball and repeat the relevant deploy steps in Section 8.
# Then restart only the affected service:
# - backend/console: docker compose -f docker-compose.hetzner.yml up -d --build <svc>
# - website: pm2 restart website

# Verify
curl https://api.apexmediation.ee/health
```

### 9.2 Database Rollback

```bash
# SSH to DB server
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35

# Restore from backup
sudo -u postgres psql adproject < /var/backups/postgres/<backup_file>.sql
```

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
