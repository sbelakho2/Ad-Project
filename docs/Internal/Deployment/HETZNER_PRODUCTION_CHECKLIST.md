# Hetzner Production Checklist

This is the production deployment runbook for ApexMediation on Hetzner infrastructure. It consolidates server setup, service deployment, database configuration, TLS termination, monitoring, and operational procedures.

> **Infrastructure Overview:**  
> - **App Server:** 46.62.252.96 (apexmediation.ee)  
> - **DB Server:** 77.42.44.35  
> - **Domains:** apexmediation.ee, console.apexmediation.ee, api.apexmediation.ee  
> - **SSH Access:** `ssh -i ~/.ssh/hetzner-db-mac root@<IP>`

## Current Production Status (2025-12-14)

| Component | Status | Details |
|-----------|--------|---------|
| Backend API | ✅ Running | Port 8080, PostgreSQL/Redis/Queues UP |
| Console Frontend | ✅ Running | Port 3000, HTTP 200 |
| **Control Plane** | ✅ Running | Port 4000, `admin.apexmediation.ee` (IP-restricted) |
| PostgreSQL 16 | ✅ Running | DB Server, database: `adproject`, 139+ tables |
| Redis | ✅ Running | App Server, port 6379, password auth |
| Caddy | ✅ Running | Auto Let's Encrypt for all domains |
| Prometheus | ✅ Running | Port 9090, 7/7 targets UP |
| Grafana | ✅ Running | Port 3001 |

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
- `apexmediation.ee` → Console (port 3000)
- `console.apexmediation.ee` → Console (port 3000)
- `api.apexmediation.ee` → Backend API (port 8080)

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

# Console
curl -I https://console.apexmediation.ee
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
cd /opt/apex && docker compose exec backend npm run migrate
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

### 8.1 Deploy Backend

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Navigate to project
cd /opt/apex

# Pull latest changes
git pull origin main

# Rebuild and restart backend
docker compose build backend
docker compose up -d backend

# Verify health
curl http://localhost:8080/health
```

### 8.2 Deploy Console

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Navigate to project
cd /opt/apex

# Pull latest changes
git pull origin main

# Rebuild and restart console
docker compose build console
docker compose up -d console

# Verify health
curl http://localhost:3000
```

---

## 9. Rollback Procedure

### 9.1 Quick Rollback

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Navigate to project
cd /opt/apex

# Rollback to previous commit
git checkout HEAD~1

# Rebuild and restart
docker compose build
docker compose up -d

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
