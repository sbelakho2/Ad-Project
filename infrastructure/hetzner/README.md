# Hetzner Infrastructure Deployment

This directory contains scripts and configuration for deploying the Ad-Project to Hetzner Cloud servers.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            INTERNET                                          │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         APP SERVER (46.62.252.96)                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │    Caddy     │  │   Backend    │  │   Console    │  │    Redis     │    │
│  │  (80/443)    │  │   (8080)     │  │   (3000)     │  │   (6379)     │    │
│  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘    │
│         │                 │                 │                 │              │
│         └─────────────────┴─────────────────┴─────────────────┘              │
│                                                                              │
│  ┌──────────────┐  ┌──────────────┐                                           │
│  │   Website    │  │ Control Plane │                                           │
│  │   (3002)     │  │    (4000)     │                                           │
│  └──────────────┘  └──────────────┘                                           │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                           Private Network
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         DB SERVER (77.42.44.35)                             │
│              ⚠️ PRIVATE ONLY - NO INTERNET ACCESS ⚠️                        │
│  ┌──────────────────────────────────┐  ┌──────────────────────────────┐    │
│  │         PostgreSQL 16            │  │       Autopilot Service      │    │
│  │           (5432)                 │  │          (8090)              │    │
│  │  ┌─────────────────────────────┐ │  │  ┌────────────────────────┐  │    │
│  │  │ core   │ config │ ops      │ │  │  │ Health Checks          │  │    │
│  │  │ events │ agg               │ │  │  │ Partition Management   │  │    │
│  │  └─────────────────────────────┘ │  │  │ Retention Policy       │  │    │
│  └──────────────────────────────────┘  │  │ Metrics Aggregation    │  │    │
│                                         │  └────────────────────────┘  │    │
│                                         └──────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Server Details

| Server | IP Address | Role | Ports |
|--------|------------|------|-------|
| App Server | 46.62.252.96 | Caddy (TLS), Backend, Console, Website, Redis, Control Plane | 22, 80, 443 |
| DB Server | 77.42.44.35 | PostgreSQL 16 (+ optional Autopilot) | 22, 5432 (private) |

### Port Allocation (CRITICAL)

All application services MUST bind to their assigned localhost ports on the App Server:

| Service | Hostname | Local Port | Notes |
|---------|----------|------------|-------|
| Website | `apexmediation.ee` / `www.apexmediation.ee` | 3002 | Customer marketing site |
| Console | `console.apexmediation.ee` | 3000 | Customer dashboard (Next.js) |
| Backend API | `api.apexmediation.ee` | 8080 | Customer API (Express) |
| Control Plane (Admin) | `admin.apexmediation.ee` | 4000 | Operator-only; MUST NOT proxy to Console |

Supporting services:

| Service | Host | Port | Access |
|---------|------|------|--------|
| Redis | App Server | 6379 | Local/private only |
| PostgreSQL | DB Server | 5432 | Private network only |
| Prometheus | App Server | 9090 | Internal |
| Grafana | App Server | 3001 | Internal |

## Security Model

### DB Server (CRITICAL)
- **NO PUBLIC INTERNET ACCESS** (recommended)
- Only accepts connections from App Server (46.62.252.96)
- Firewall rules strictly limit access to:
  - SSH (port 22) - from App Server only
  - PostgreSQL (port 5432) - from App Server only
  - Autopilot API (port 8090) - from App Server only

### App Server
- Public-facing (80/443 via Caddy)
- SSH access (port 22)
- All backend services run on localhost
- Connects to DB Server via private network

## SSH Access

```bash
# App Server (public)
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# DB Server (direct access currently enabled)
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35

# Alternative: DB Server via App Server as jump host
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35
```

## Deployment Steps

### 1. Initial Setup (One-time)

#### DB Server First
```bash
# SSH to DB server via jump host
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35

# Upload setup script
scp -i ~/.ssh/hetzner-db-mac -o ProxyJump=root@46.62.252.96 infrastructure/hetzner/setup-db-server.sh root@77.42.44.35:/root/

# Run setup
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35 "chmod +x /root/setup-db-server.sh && /root/setup-db-server.sh"
```

#### App Server Second
```bash
# SSH to App server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Upload and run setup
scp -i ~/.ssh/hetzner-db-mac infrastructure/hetzner/setup-app-server.sh root@46.62.252.96:/root/
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "chmod +x /root/setup-app-server.sh && /root/setup-app-server.sh"
```

### 2. Run Database Migrations

```bash
# SSH to DB server
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35

# Run migrations
sudo -u postgres psql -d adproject -f /opt/autopilot/migrations/034_hetzner_schema_reorganization.sql
```

### 3. Deploy Autopilot Service

```bash
# Build autopilot locally
cd services/autopilot
pnpm install
pnpm build

# Upload to DB server (via App server)
tar -czf autopilot.tar.gz dist package.json .env.example
scp -i ~/.ssh/hetzner-db-mac -o ProxyJump=root@46.62.252.96 autopilot.tar.gz root@77.42.44.35:/opt/autopilot/

# Extract and start
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35 << 'EOF'
cd /opt/autopilot
tar -xzf autopilot.tar.gz
npm install --production
systemctl start autopilot
systemctl enable autopilot
systemctl status autopilot
EOF
```

### 4. Deploy Backend & Console

```bash
# SSH to App server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Deploy backend + console via Docker Compose
cd /opt/adproject
docker compose -f infrastructure/docker/docker-compose.hetzner.yml up -d --build backend console

# Control Plane runs separately (systemd) on port 4000
systemctl status adproject-control-plane || true

# Caddy is the edge router (TLS termination)
sudo caddy reload --config /etc/caddy/Caddyfile
```

### 5. Setup SSL Certificates

```bash
# TLS is handled by Caddy automatically (Let's Encrypt)
sudo caddy reload --config /etc/caddy/Caddyfile
```

## Database Schemas

The database uses 5 schemas for workload separation:

| Schema | Purpose | Tables |
|--------|---------|--------|
| `core` | Business entities | publishers, apps, placements, adapters, users, vra_ledger |
| `config` | Runtime configuration | feature_flags, ab_tests, adapter_configs, migration_experiments |
| `ops` | Operational data | autopilot_status, autopilot_jobs, audit_logs, billing_audit |
| `events` | High-volume events | ad_requests, bid_responses, impressions, clicks, revenue |
| `agg` | Pre-computed aggregates | metrics_hourly, metrics_daily, metrics_monthly |

## Autopilot Service

The Autopilot service runs on the DB server and provides:

- **Health Monitoring**: Connection usage, cache hit ratio, long queries, bloat detection
- **Partition Management**: Auto-creates future partitions, drops old ones per retention policy
- **Metrics Aggregation**: Computes hourly/daily metrics from events tables
- **REST API**: Exposes health and control endpoints for Admin Control Panel

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Basic health check |
| `/status` | GET | Full autopilot status |
| `/metrics` | GET | Database metrics |
| `/metrics/prometheus` | GET | Prometheus format metrics |
| `/jobs` | GET | Recent autopilot jobs |
| `/jobs/trigger/:type` | POST | Trigger job manually |
| `/partitions` | GET | Partition info |
| `/partitions/ensure` | POST | Create future partitions |
| `/partitions/cleanup` | POST | Run retention cleanup |
| `/actions/:action` | POST | Execute safe action |

## Monitoring

### Health Checks

```bash
# Backend health (App server)
curl http://127.0.0.1:8080/health

# Console (App server)
curl -I http://127.0.0.1:3000

# Control Plane (App server)
curl -I http://127.0.0.1:4000/health

# DB server status (from App server)
curl http://77.42.44.35:8090/status
```

### Logs

- **Backend**: `/var/log/adproject/backend.log`
- **Console**: `/var/log/adproject/console.log`
- **Caddy**: `/var/log/caddy/access.log` (or `journalctl -u caddy -f`)
- **Autopilot**: `/var/log/autopilot/stdout.log`
- **PostgreSQL**: `/var/log/postgresql/postgresql-16-main.log`

### Admin Control Panel

Access the operator Control Plane at:
```
https://admin.apexmediation.ee/admin
```

Features:
- Real-time autopilot status
- Connection usage metrics
- Table statistics
- Partition management
- Manual job triggers

## Maintenance

### Daily Tasks (Automated by Autopilot)
- Hourly metrics aggregation
- Daily metrics aggregation
- Partition creation for next 7 days

### Weekly Tasks (Automated by Autopilot)
- Old job cleanup (30+ days)
- Retention policy enforcement

### Manual Tasks
- Review Autopilot status in Admin panel
- Check for DEGRADED/FAIL states
- Review slow query logs

## Troubleshooting

### DB Connection Issues
```bash
# Check DB server firewall
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35 "ufw status"

# Test PostgreSQL connection from App server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "psql -h 77.42.44.35 -U adproject -d adproject -c 'SELECT 1'"
```

### Autopilot Not Running
```bash
# Check service status
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35 "systemctl status autopilot"

# View logs
ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35 "tail -100 /var/log/autopilot/stdout.log"
```

### High Connection Usage
```bash
# View active connections
sudo -u postgres psql -c "SELECT count(*), state FROM pg_stat_activity GROUP BY state"

# Kill idle connections (via Autopilot)
curl -X POST http://77.42.44.35:8090/actions/kill_idle_connections
```

## Rollback Procedure

1. Stop services on App server
2. Restore database from backup
3. Deploy previous code version
4. Restart services

## Backup Strategy

- PostgreSQL: Daily pg_dump to external storage
- Redis: RDB snapshots (configured)
- Code: Git repository (versioned)
