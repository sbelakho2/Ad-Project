# Infrastructure Migration Plan — COMPLETED (Now on Hetzner)

> **MIGRATION COMPLETE (2025-12-13)**: The platform has fully migrated from DigitalOcean to Hetzner infrastructure.
>
> **Current Production Infrastructure:**
> - **App Server:** 203.0.113.10 (apexmediation.ee)
> - **DB Server:** 203.0.113.12
> - **Domains:** apexmediation.ee, console.apexmediation.ee, api.apexmediation.ee
> - **SSH Access:** `ssh -i ~/.ssh/hetzner-db-mac root@<IP>`
> - **TLS:** Caddy with auto Let's Encrypt
> - **Database:** PostgreSQL 16 (self-hosted on DB server)
> - **Cache:** Redis on app server with password authentication
>
> **See:** [HETZNER_PRODUCTION_CHECKLIST.md](../Deployment/HETZNER_PRODUCTION_CHECKLIST.md) for current operations guide.

---

## Historical Context (DigitalOcean Plan - SUPERSEDED)

The content below documents the original DigitalOcean migration plan. This has been superseded by the Hetzner deployment, but is retained for historical reference.

---

## Original Plan: DigitalOcean, $50/mo Max (FRA1 + TLS hardened)
### Goal: Production-ready under $50/month, minimal overhead

This plan replaces the previous multi-provider direction (Supabase + Upstash + ClickHouse) with a single DigitalOcean droplet, DO Managed Postgres, self-hosted Redis, and MinIO (self-hosted S3) for objects and backups with optional offsite sync to DigitalOcean Spaces. Monitoring leans on DO built-ins with an optional lightweight Grafana stack. ClickHouse is deferred; use Postgres-first analytics. Region is pinned to FRA1 (Frankfurt) by default for EU latency/compliance. TLS is terminated on the droplet via Let’s Encrypt with hardened defaults (HSTS after validation, OCSP stapling, modern ciphers).

Budget ceiling: $50/month (target $44–49/month)

---

## 1. Core Infrastructure (DigitalOcean-centric, ≈ $40–45/mo)
### 1.1 Compute — Main App Droplet
Goal: one solid VPS that runs API, console, background jobs, Redis and light observability.

- Create DigitalOcean account
- Create droplet `apex-core-1`
  - Type: Basic Regular Intel/AMD or Premium AMD/Intel
  - Size: 2 vCPU / 4 GB RAM / 80 GB SSD (≈ $24/mo)
  - OS: Ubuntu LTS (e.g., 22.04)
  - Region: FRA1 (Frankfurt) — default. Only dev/test may use other regions if required.
- Base server hardening
  - Create non-root user, disable password SSH, allow only key auth
  - Enable UFW: allow 22/tcp (SSH), 80/tcp (HTTP), 443/tcp (HTTPS); deny everything else
  - Install fail2ban (or similar)
  - Keep automatic security updates enabled (unattended-upgrades)
- Runtime setup (Docker or bare metal)
  - Install Docker + docker-compose (or containerd)
  - Define docker-compose.yml for:
    - api service (Go/Node/etc.)
    - console service (Next.js/React frontend)
    - redis service
    - nginx reverse proxy (if not using app server directly)
  - Expose only nginx on 80/443; keep app containers internal
- App networking
  - Use DigitalOcean Load Balancer (optional, later) – for now, single droplet
  - Configure Nginx:
    - `api.apexmediation.ee` → backend API
    - `console.apexmediation.ee` → frontend console
    - `status.apexmediation.ee` reserved (for status page later)

### 1.2 Database — Managed PostgreSQL (Primary DB + Light Analytics)
Goal: managed, low-maintenance Postgres; use it for core data + early analytics; defer ClickHouse until needed.

- Create DigitalOcean Managed PostgreSQL cluster
  - Size: smallest Basic/Development plan that supports your workload (≈ $15/mo)
  - Region: FRA1 (same as droplet `apex-core-1`) to minimize latency and keep data in EU
  - Storage: 10–20 GB to start
- Security & access
  - Restrict DB access to:
    - apex-core-1 droplet private IP
    - Optional admin IP (your VPN/home IP) via firewall rule
  - Enable encrypted connections (SSL) and enforce SSL from apps
- Create DB roles:
  - `apex_app` (limited privileges)
  - `apex_admin` (migration / maintenance)
- Store DB credentials in:
  - Env vars + encrypted secrets (DO App Secrets or operator vault like KeePassXC/`pass`)
- Schema & migrations
  - Port existing migrations (001–008+) to run against managed Postgres
  - Use a migration tool (Flyway/Liquibase/Prisma/custom) in CI: one command `deploy:migrations` for prod
  - Confirm:
    - All tables created
    - Indices on hot columns (e.g., app_id, placement_id, publisher_id, timestamp)
    - Foreign keys & constraints correct
- Backups & retention
  - Enable automated daily backups (DO setting)
  - Test point-in-time recovery (PITR) in a staging DB: restore to a new instance and verify schema & data
  - Document RPO/RTO:
    - RPO: 24h or better
    - RTO: 1–4h for full restore
- Analytics (Postgres first)
  - Create aggregated analytics tables:
    - `daily_app_metrics` (by app, date, country, format)
    - `daily_network_metrics`
  - Use Postgres for early analytics dashboards (avoid ClickHouse until volume demands it)
  - Connection security:
    - Require SSL from all clients (`sslmode=require`)
    - Example connection URL: `postgresql://apex_app:<PASSWORD>@<DO_PG_HOST>:25060/ad_platform?sslmode=require`
    - If using certificate pinning, download DO CA and configure the client accordingly

### 1.3 Cache — Redis (Self-Hosted on the Droplet)
Goal: simple in-memory cache for rate limits, short-lived state, feature flags; avoid Upstash cost.

- Install Redis via Docker (`redis:6-alpine` or newer) or APT
- Bind Redis to 127.0.0.1 (or Docker internal network) – no public exposure
- Configure Redis:
  - Set max memory limit (e.g., 512 MB) and eviction policy (`allkeys-lru`)
  - Enable authentication (`requirepass`) even if local
  - Turn on append-only file (AOF) or RDB snapshots depending on persistence needs
- Use Redis for:
  - Rate limiting
  - Idempotency keys
  - Short-lived feature flags / config cache
  - Not for long-term analytics (that’s Postgres)

### 1.4 TLS Termination & Security (Let’s Encrypt, hardened)
Goal: always-on HTTPS with modern security defaults; optional mTLS for sensitive paths.

- DNS: point `api.apexmediation.ee` and `console.apexmediation.ee` A/AAAA to the droplet public IP
- Install certbot and obtain certificates for both hosts:
  ```bash
  apt-get update && apt-get install -y certbot python3-certbot-nginx
  certbot --nginx -d api.apexmediation.ee -d console.apexmediation.ee --redirect --email security@apexmediation.ee --agree-tos
  ```
- Harden Nginx TLS (example snippet):
  ```nginx
  # /etc/nginx/snippets/ssl-params.conf
  ssl_protocols TLSv1.2 TLSv1.3;
  ssl_prefer_server_ciphers on;
  ssl_ciphers 'ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256';
  ssl_session_timeout 1d;
  ssl_session_cache shared:SSL:10m;
  ssl_session_tickets off;
  ssl_stapling on;
  ssl_stapling_verify on;
  resolver 1.1.1.1 1.0.0.1 valid=300s;
  add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
  add_header X-Content-Type-Options nosniff;
  add_header X-Frame-Options SAMEORIGIN;
  add_header Referrer-Policy no-referrer-when-downgrade;
  add_header X-XSS-Protection "1; mode=block";
  ```
- Update Nginx server blocks to listen on 443 with the issued certs and include `ssl-params.conf`.
- Containerization note (if Nginx runs in Docker):
  - Mount certs and snippets into the container read-only:
    - In `infrastructure/docker-compose.prod.yml`, expose 443 and add:
      - `- /etc/letsencrypt:/etc/letsencrypt:ro`
      - `- ./nginx/snippets:/etc/nginx/snippets:ro`
  - Reference certbot paths inside the container, e.g. `/etc/letsencrypt/live/api.apexmediation.ee/fullchain.pem`.
- Cert renewals: certbot installs a systemd timer/cron for auto-renew; verify with `certbot renew --dry-run`.
- Optional mTLS for internal endpoints (example `/metrics`): create a dedicated server/location with `ssl_verify_client on;` and trusted client CA bundle.

Metrics endpoint protection (optional)
- Basic Auth: A ready-made snippet is provided at `infrastructure/nginx/snippets/metrics-basic-auth.conf`.
  - Create credentials on the droplet: `mkdir -p /opt/apex/infrastructure/nginx/htpasswd && htpasswd -c /opt/apex/infrastructure/nginx/htpasswd/metrics admin`
  - Ensure the compose file mounts the directory (already present): `./nginx/htpasswd:/etc/nginx/htpasswd:ro`
  - In `apexmediation.conf`, inside the `/metrics` location, uncomment `include /etc/nginx/snippets/metrics-basic-auth.conf;`.
  - Reload Nginx container.
- Alternative: mTLS for `/metrics` later.

### 1.5 Object Storage & Backups
Goal: cheap, durable object storage for invoices, reports, and long-term backups.



DigitalOcean Spaces (simple, tightly integrated)
- Create a Spaces bucket (`apex-prod-objects`) (≈ $5/mo)
- Use for:
  - PDFs and HTML exports (invoices, Proof-of-Revenue digest reports)
  - Generated CSVs / zipped dispute kits (VRA)
- Apply:
  - Private by default, with signed URLs for downloads
  - Lifecycle rules for intermediate artifacts (delete after 30–90 days)


### 1.6 Budget Check
Approximate monthly:
- Droplet 2 vCPU / 4GB: $24
- Managed Postgres basic: $15
- Spaces primary + optional S3-compatible mirror: $5
- Misc (egress, DNS, backup overhead): $3–5
👉 Total: $44–49 / month, within your $50 target.

---

## 2. Monitoring, Logging & Error Tracking (Solo-friendly, Low Cost)

### 2.1 Basic Host & App Monitoring — DigitalOcean Monitoring
- Enable DO monitoring for `apex-core-1`: CPU, RAM, Disk, Network metrics
- Configure alerts:
  - CPU > 80% for 5 mins
  - Memory > 80% for 5 mins
  - Disk > 80% usage
  - Droplet unreachable

### 2.1.b Lightweight Grafana Option (Optional)
If you want rich graphs and can spare some RAM:
- Run Prometheus + Grafana in Docker on `apex-core-1`
- Prometheus scraping:
  - Node exporter (system metrics)
  - App metrics endpoints (`/metrics` from API)
- Grafana:
  - System dashboard
  - API performance dashboard (p95 latency, error rate)
  - Business metrics (DAU, MRR, usage, mediated revenue)
- Set retention low (7–30 days) to conserve storage
- Protect Grafana with: Auth + IP restriction (or VPN)
- If this feels heavy, skip self-hosting initially and use DO built-in charts + hosted Grafana Cloud free tier.

### 2.2 Application Logging
- Standardize on structured logs (JSON) from backend and services:
  - Fields: timestamp, level, service, request_id, user_id, app_id, path, latency_ms, error_code
- For now:
  - Stream logs to file + DO console
  - Add log rotation (logrotate) to keep disk usage in check
- Optional: add Loki (self-hosted) later if log volume grows

### 2.3 Error Tracking
- Create Sentry account (free tier)
- Integrate Sentry into:
  - Backend API
  - Console (frontend)
- Configure:
  - Release version tagging
  - Environment tags (dev, staging, prod)
  - Basic sanitization (no PII in logs/errors)
- For SDK crash reporting on clients:
  - Use Firebase Crashlytics for Android & iOS
  - Ensure mapping/scopes correct (no user identifiers beyond what’s necessary)

### 2.4 Status Page
To avoid self-hosting for now:
- Use Upptime (GitHub-based) or a free/cheap service like UptimeRobot / Better Stack
- Monitor:
  - https://api.apexmediation.ee/health
  - https://console.apexmediation.ee/
- Configure:
  - Friendly names and status messages
  - Public status page: `status.apexmediation.ee` CNAME → service / GitHub Pages
- Test:
  - Simulate outage by stopping API → ensure status turns red + notification is sent

### 2.5 Alerting (Solo-Founder Reality)
- Use PagerDuty Free, Better Stack alerts, or just email/SMS from monitoring
- Set up:
  - Critical alerts:
    - API health endpoint down
    - DB connectivity loss
    - Error rate > X% for Y minutes
  - Warning alerts:
    - High latency
    - Queue/backlog growth
- Escalation policy:
  - For now: you only — Email + SMS
- Test:
  - Trigger a test alert (force failing `/health`) and confirm:
    - Email arrives
    - SMS/push arrives (if configured)

---

## 3. “Works Out of the Box” Checklist
Final pass to ensure the infra/monitoring stack does what you need with minimal babysitting.

### 3.1 Connectivity & Routing
- `api.apexmediation.ee` resolves and serves HTTPS (valid cert)
- `console.apexmediation.ee` resolves and loads app
- All API endpoints used by console reachable from browser (CORS OK)
- SSH access via key only works; root login disabled

### 3.2 App + DB
- API can connect to DO Managed Postgres (correct host, SSL, credentials)
- Migrations run cleanly in prod
- Basic flows tested end-to-end:
  - Signup
  - App creation
  - Placement configuration
  - SDK key generation
  - Data appears in DB & dashboards

### 3.3 Redis & Caching
- Redis accessible from API only, not from public internet
- Cache used for at least one visible performance path (e.g., reading static configs)
- Failover behaviour: If Redis is down, app degrades gracefully (no hard crash)

### 3.4 Monitoring & Alerts
- At least one Grafana or DO dashboard shows: CPU, RAM, Disk, network; API latency, QPS, error rate
- Sentry issues arrive for forced error in dev (test endpoint)
- Status page reflects downtime when API `/health` returns 500 or times out
- At least one alert has been triggered & acknowledged successfully

### 3.5 Backups & Recovery
- DO Postgres backups are enabled
- Tested: Restore DB snapshot to new instance in staging; app pointed to new DB works
- Object storage:
  - File upload/download tested (e.g., dummy invoice)
  - Bucket permissions correct (private; signed URLs only)

---

## TL;DR
Move from Supabase + Upstash + ClickHouse to DigitalOcean droplet + DO Managed Postgres + simple Redis + Spaces (plus optional offsite mirror).

Lean heavily on:
- DO monitoring,
- Sentry,
- a simple status page,
- very small self-hosted observability where needed.

Your infra cost stays under $50/month, with enough horsepower for an early production BYO mediation+VRA platform, and low mental overhead for a solo operator.

### Phase 1: Core Infrastructure Migration (Week 1-2)
**Goal**: Migrate primary services to cost-effective alternatives
**Cost Impact**: -$400-800/month

#### 1.1 Application Hosting: DigitalOcean Droplet
**From**: AWS EC2 ($100-300/month) or Heroku ($50-250/month)
**To**: DigitalOcean Droplet (2 vCPU / 4 GB RAM / 80 GB SSD) (~$24/month)

**Migration Steps**:
```bash
# 1. Provision droplet
# DO Console → Create Droplet → Ubuntu 22.04 LTS, 2 vCPU / 4GB / 80GB, region FRA/AMS/NYC

# 2. Harden host (SSH into droplet as root)
adduser deploy && usermod -aG sudo deploy
rsync -av ~/.ssh/authorized_keys deploy@<droplet-ip>:/home/deploy/.ssh/
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config && systemctl restart ssh
ufw allow 22 && ufw allow 80 && ufw allow 443 && ufw --force enable
apt-get update && apt-get install -y fail2ban unattended-upgrades

# 3. Install Docker & docker-compose
curl -fsSL https://get.docker.com | bash
usermod -aG docker deploy

# 4. Clone repo and configure
sudo -u deploy mkdir -p /opt/apex && sudo -u deploy git clone <repo-url> /opt/apex
cd /opt/apex

# 5. Configure environment
cp backend/.env.example backend/.env
# Set DATABASE_URL to DO Managed Postgres with sslmode=require; set REDIS_URL to internal redis

# 6. Start stack (only Nginx exposed in production; for dev map ports as needed)
docker compose -f docker-compose.yml up -d backend console redis

# 7. Install and configure Nginx as reverse proxy
apt-get install -y nginx
cat >/etc/nginx/sites-available/apexmediation.conf <<'NG'
server {
  listen 80;
  server_name api.apexmediation.ee;
  location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
  }
}
server {
  listen 80;
  server_name console.apexmediation.ee;
  location / {
    proxy_pass http://127.0.0.1:3000;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
  }
}
NG
ln -s /etc/nginx/sites-available/apexmediation.conf /etc/nginx/sites-enabled/
systemctl restart nginx

# 8. Issue certificates (MANDATORY for prod)
# Use certbot on the droplet to terminate TLS locally; Cloudflare can remain DNS-only (no proxy) or be configured for Full (strict) if used.
apt-get update && apt-get install -y certbot python3-certbot-nginx
certbot --nginx -d api.apexmediation.ee -d console.apexmediation.ee --redirect --email security@apexmediation.ee --agree-tos
certbot renew --dry-run
```

**Verification**:
```bash
curl -I http://api.apexmediation.ee/health
curl -I http://console.apexmediation.ee/
docker ps --format 'table {{.Names}}\t{{.Status}}'
```

**Rollback Plan**:
- Keep old infrastructure running during migration
- Dual-run for 1 week with traffic split
- Full rollback if latency >200ms or errors >0.5%

---

#### 1.2 Database: DigitalOcean Managed PostgreSQL
Use DigitalOcean Managed Postgres (Basic/Dev tier) in the same region as the droplet with SSL required.

Key steps (see section 1.2 above for details):
- Create cluster (10–20GB to start), restrict access to droplet private IP.
- Create roles `apex_app` and `apex_admin`; store credentials in DO App Secrets and mirror into operator vault (KeePassXC/`pass`).
- Run migrations via CI task `deploy:migrations`; verify tables, indexes, FKs.
- Enable backups and perform a PITR drill; document RPO/RTO.

---

#### 1.3 Analytics: Postgres-first (ClickHouse deferred)
Policy: Always prefer Postgres over 3rd-party analytics services until clear scale signals. ClickHouse is deferred and removed from this plan. Build early aggregates in Postgres and add indexes/materialized views as needed.

Recommended steps:
- Create `daily_app_metrics` and `daily_network_metrics` tables with concise schemas and appropriate composite indexes.
- Use materialized views and scheduled refresh (cron/systemd timer) for heavy reports.
- Keep long-term raw event storage minimal; archive to MinIO (primary) as gzip CSV/Parquet; optionally mirror offsite to DigitalOcean Spaces on a weekly schedule.
- Only revisit ClickHouse if Postgres p95 query latency for dashboards exceeds SLOs after optimization.

---

#### 1.4 Cache: Self-hosted Redis (no 3rd-party)
Policy: Always use self-hosted Redis on the droplet for ephemeral caching (rate limits, idempotency, flags). Do not use Upstash/Redis Cloud.

Operational notes:
- Bind to localhost or Docker internal network; require password; set maxmemory 512MB + `allkeys-lru`.
- Persistence: AOF or snapshots depending on need; for pure cache, RDB snapshots daily are sufficient.
- No migration required for cache keys; cold start is acceptable.

---

### Phase 2: Monitoring & Observability (Week 3)
**Goal**: Replace Datadog ($50-200/month) with a lightweight, self-hosted stack on the droplet (optional) or use DO built-ins
**Cost Impact**: -$50-200/month

#### 2.1 Self-Hosted Monitoring: Grafana + Prometheus + Loki
**From**: Datadog ($50-200/month)
**To**: Self-hosted on the DigitalOcean droplet ($0 incremental)

**Architecture**:
```
┌─────────────────┐
│  Backend VMs    │──metrics──▶┌──────────────┐
│  (Express.js)   │            │  Prometheus  │──query──▶┌──────────┐
└─────────────────┘            │  (scraper)   │          │ Grafana  │
                               └──────────────┘          │  (UI)    │
┌─────────────────┐                                      └──────────┘
│  Console VMs    │──logs────▶┌──────────────┐                ▲
│  (Next.js)      │            │     Loki     │────query───────┘
└─────────────────┘            │ (log store)  │
                               └──────────────┘
```

**Setup**:
```bash
# 1. Prepare monitoring stack directory on the droplet
mkdir -p /opt/monitoring && cd /opt/monitoring

# 2. Create docker-compose.yml for monitoring stack
cat > docker-compose.yml <<EOF
version: '3.8'
services:
  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - prometheus-data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.retention.time=30d'
  
  loki:
    image: grafana/loki:latest
    ports:
      - "3100:3100"
    volumes:
      - ./loki-config.yml:/etc/loki/local-config.yaml
      - loki-data:/loki
  
  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=\${GRAFANA_PASSWORD}
      - GF_INSTALL_PLUGINS=grafana-piechart-panel
    volumes:
      - grafana-data:/var/lib/grafana
      - ./grafana-datasources.yml:/etc/grafana/provisioning/datasources/datasources.yml

volumes:
  prometheus-data:
  loki-data:
  grafana-data:
EOF

# 3. Configure Prometheus scraping
cat > prometheus.yml <<EOF
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'backend'
    static_configs:
      - targets:
        - '127.0.0.1:8080'
        metrics_path: '/metrics'
  
  - job_name: 'console'
    static_configs:
      - targets:
        - '127.0.0.1:3000'
        metrics_path: '/metrics'
EOF

# 4. Deploy monitoring stack
docker compose up -d

# 5. Access Grafana
echo "Open http://<droplet-ip>:3000 (admin / change via GF_SECURITY_ADMIN_PASSWORD)."

# 6. Import dashboards
# Backend API: Dashboard ID 11159 (Node.js Application)
# PostgreSQL: Dashboard ID 9628 (PostgreSQL Database)
# System: Dashboard ID 1860 (Node Exporter Full)
```

---

#### 2.2 Error Tracking: Self-Hosted GlitchTip (Optional)
**From**: Sentry paid tier ($26-80/month)
**To**: GlitchTip self-hosted on the droplet ($0 incremental)

**GlitchTip Features**:
- Unlimited events (vs Sentry's 100K/month limit)
- Same SDKs as Sentry (drop-in replacement)
- Error tracking, performance monitoring, uptime checks
- Open source (MIT license)

**Setup**:
```bash
# 1. Prepare GlitchTip directory on the droplet
mkdir -p /opt/glitchtip && cd /opt/glitchtip

# 2. Create docker-compose.yml
cat > docker-compose.yml <<EOF
version: '3.8'
services:
  postgres:
    image: postgres:15
    environment:
      POSTGRES_DB: glitchtip
      POSTGRES_USER: glitchtip
      POSTGRES_PASSWORD: \${DB_PASSWORD}
    volumes:
      - postgres-data:/var/lib/postgresql/data
  
  redis:
    image: redis:7-alpine
  
  web:
    image: glitchtip/glitchtip:latest
    depends_on:
      - postgres
      - redis
    ports:
      - "8000:8000"
    environment:
      DATABASE_URL: postgres://glitchtip:\${DB_PASSWORD}@postgres/glitchtip
      REDIS_URL: redis://redis:6379/0
      SECRET_KEY: \${SECRET_KEY}
      PORT: 8000
      EMAIL_URL: smtp://localhost:587
      GLITCHTIP_DOMAIN: https://errors.apexmediation.ee
      DEFAULT_FROM_EMAIL: support@apexmediation.ee

volumes:
  postgres-data:
EOF

# 3. Deploy GlitchTip
export SECRET_KEY=$(openssl rand -hex 32)
export DB_PASSWORD=$(openssl rand -hex 16)
docker compose up -d

# 4. Initialize GlitchTip
echo "Open https://errors.apexmediation.ee and complete setup."
# Create admin account
# Create organization "ApexMediation"
# Create project "Backend"

# 5. Get DSN (Data Source Name)
# Settings → Client Keys (DSN) → Copy DSN

# 6. Update backend to use GlitchTip
npm install @sentry/node # GlitchTip uses Sentry SDK
# backend/lib/errors.ts
import * as Sentry from '@sentry/node';
Sentry.init({
  dsn: process.env.GLITCHTIP_DSN,
  environment: process.env.NODE_ENV,
  tracesSampleRate: 1.0,
});

# 7. Update environment variables
export GLITCHTIP_DSN=$GLITCHTIP_DSN

# 8. Test error reporting
curl -X POST https://api.apexmediation.ee/api/test/error
# Check GlitchTip dashboard for error event
```

---

### Phase 3: Email & Marketing Infrastructure (Week 4)
**Goal**: Replace expensive SaaS tools with self-hosted alternatives
**Cost Impact**: -$300-500/month

#### 3.1 Email Marketing: Listmonk
**From**: Mailchimp ($350-500/month for 10K-50K subscribers)
**To**: Listmonk self-hosted on the droplet ($0 incremental) + Postfix for delivery (unlimited emails)

**Listmonk Features**:
- Unlimited subscribers (vs Mailchimp's tiered pricing)
- Email campaigns, newsletters, transactional emails
- List management, segmentation, analytics
- Open source (AGPL license)

**Setup**:
```bash
# 1. Prepare Listmonk directory on the droplet
mkdir -p /opt/listmonk && cd /opt/listmonk

# 2. Create docker-compose.yml
cat > docker-compose.yml <<EOF
version: '3.8'
services:
  postgres:
    image: postgres:15
    environment:
      POSTGRES_DB: listmonk
      POSTGRES_USER: listmonk
      POSTGRES_PASSWORD: \${DB_PASSWORD}
    volumes:
      - postgres-data:/var/lib/postgresql/data
  
  app:
    image: listmonk/listmonk:latest
    depends_on:
      - postgres
    ports:
      - "9000:9000"
    environment:
      TZ: America/Los_Angeles
    volumes:
      - ./config.toml:/listmonk/config.toml

volumes:
  postgres-data:
EOF

# 3. Create config.toml
cat > config.toml <<EOF
[app]
address = "0.0.0.0:9000"
admin_username = "admin"
admin_password = "$LISTMONK_PASSWORD"

[db]
host = "postgres"
port = 5432
user = "listmonk"
password = "$DB_PASSWORD" # Provided via environment/secrets manager
database = "listmonk"
ssl_mode = "disable"

# Use self-hosted Postfix for email delivery
[[messengers]]
enabled = true
host = "mail.apexmediation.ee"
port = 587
auth_protocol = "plain"
username = "" # No auth needed for local Postfix
password = "" # No auth needed for local Postfix
email_format = "html"
max_conns = 10
idle_timeout = "15s"
wait_timeout = "5s"
max_msg_retries = 2
EOF

# 4. Deploy Listmonk
export DB_PASSWORD=$(openssl rand -hex 16)
export LISTMONK_PASSWORD=$(openssl rand -hex 16)
docker compose up -d

# 5. Initialize Listmonk
echo "Open https://listmonk.apexmediation.ee and complete setup."
# Login: admin / $LISTMONK_PASSWORD
# Setup → SMTP Settings → Test Connection → ✓

# 6. Import subscriber lists
# Subscribers → Import → Upload CSV (email, name, attributes)

# 7. Create first campaign
# Campaigns → New Campaign → Name: "Welcome Series" → Schedule
```

---

#### 3.2 Workflow Automation: n8n
**From**: Zapier ($19-249/month)
**To**: n8n self-hosted on the droplet ($0 incremental)

**n8n Features**:
- 280+ integrations (Mollie, Slack, GitHub, email, databases)
- Visual workflow builder
- Webhooks, cron triggers, error handling
- Open source (Fair Code license)

**Setup**:
```bash
# 1. Prepare n8n directory on the droplet
mkdir -p /opt/n8n && cd /opt/n8n

# 2. Create docker-compose.yml
cat > docker-compose.yml <<EOF
version: '3.8'
services:
  n8n:
    image: n8nio/n8n:latest
    restart: unless-stopped
    ports:
      - "5678:5678"
    environment:
      - N8N_PORT=5678
      - N8N_PROTOCOL=https
      - N8N_HOST=workflows.apexmediation.ee
      - WEBHOOK_URL=https://workflows.apexmediation.ee
      - GENERIC_TIMEZONE=UTC
    volumes:
      - n8n_data:/home/node/.n8n
volumes:
  n8n_data:
EOF

# 3. Deploy n8n
docker compose up -d

# 4. Access n8n
echo "Open https://workflows.apexmediation.ee to complete setup (create owner account)."

# 6. Example workflows to create:
# - Workflow 1: New Customer → Send Welcome Email + Create Mollie Customer
# - Workflow 2: Payment Failed → Update Status + Send Dunning Email + Alert Founder
# - Workflow 3: Usage Milestone → Celebrate + Track in ClickHouse + Notify Team
# - Workflow 4: GitHub Issue → Create Support Ticket + Notify in Discord
# - Workflow 5: Daily Report → Aggregate Stats + Email Founder + Post in Slack
```

---

### Phase 4: Status Page & Uptime Monitoring (Week 4)
**Goal**: Replace UptimeRobot paid tier ($58/month) with free alternatives
**Cost Impact**: -$58/month

#### 4.1 Status Page: Upptime (GitHub Pages)
**From**: UptimeRobot paid tier ($58/month) or Statuspage.io ($29-99/month)
**To**: Upptime on GitHub Pages ($0/month)

**Upptime Features**:
- Hosted on GitHub Pages (free, fast, reliable)
- Automated uptime monitoring via GitHub Actions
- Status page updates automatically
- Historical data (90-day uptime percentage)
- Incident management
- Open source (MIT license)

**Setup**:
```bash
# 1. Fork Upptime repository
# Visit https://github.com/upptime/upptime
# Click "Use this template" → "Create a new repository"
# Repository name: "status" (will become status.apexmediation.ee)

# 2. Configure .upptimerc.yml
cat > .upptimerc.yml <<EOF
owner: apexmediation
repo: status
user-agent: upptime

sites:
  - name: Backend API
    url: https://api.apexmediation.ee/health
    expectedStatusCodes:
      - 200
  
  - name: Console
    url: https://console.apexmediation.ee
    expectedStatusCodes:
      - 200
  
  - name: Documentation
    url: https://docs.apexmediation.ee
    expectedStatusCodes:
      - 200
  
  - name: SDK Download (iOS)
    url: https://cdn.apexmediation.ee/sdk/ios/latest/ApexSDK.framework.zip
    expectedStatusCodes:
      - 200
  
  - name: SDK Download (Android)
    url: https://cdn.apexmediation.ee/sdk/android/latest/apex-sdk.aar
    expectedStatusCodes:
      - 200

status-website:
  cname: status.apexmediation.ee
  logoUrl: https://apexmediation.ee/logo.png
  name: ApexMediation Status
  introTitle: "**Real-time status** and uptime monitoring"
  introMessage: All systems operational
  navbar:
    - title: Status
      href: /
    - title: GitHub
      href: https://github.com/apexmediation

# Notifications
notifications:
  - type: slack
    webhook-url: \${{ secrets.SLACK_WEBHOOK_URL }}
  
workflowSchedule:
  graphs: "0 0 * * *"
  responseTime: "0 23 * * *"
  staticSite: "0 1 * * *"
  summary: "0 0 * * *"
  updateTemplate: "0 0 * * *"
  updates: "0 3 * * *"
  uptime: "*/5 * * * *" # Check every 5 minutes

# Commit messages
commitMessages:
  readmeContent: ":pencil: Update summary in README [skip ci]"
  summaryJson: ":card_file_box: Update status summary [skip ci]"
  statusChange: "\${{ env.SITE_NAME }} is \${{ env.STATUS }}"
  graphsUpdate: ":bento: Update graphs [skip ci]"
EOF

# 3. Set GitHub secrets
# Settings → Secrets and variables → Actions → New repository secret
# Name: SLACK_WEBHOOK_URL, Value: https://hooks.slack.com/services/...

# 4. Enable GitHub Pages
# Settings → Pages → Source: "GitHub Actions"
# Custom domain: status.apexmediation.ee

# 5. Add DNS records
# CNAME status.apexmediation.ee → apexmediation.github.io

# 6. Wait for first run
# Actions tab → Wait for workflows to complete (~5 minutes)

# 7. Visit status page
# https://status.apexmediation.ee

# 8. Subscribe to updates
# RSS feed: https://status.apexmediation.ee/history/rss.xml
# Atom feed: https://status.apexmediation.ee/history/atom.xml
```

---

### Phase 5: Analytics & Tracking (Week 5)
**Goal**: Replace Google Analytics with privacy-focused, self-hosted alternative
**Cost Impact**: $0 (self-hosted on existing infrastructure)

#### 5.1 Website Analytics: Umami or Plausible
**From**: Google Analytics (free but privacy concerns)
**To**: Umami self-hosted on the droplet ($0 incremental)

**Umami Features**:
- Privacy-focused (no cookies, GDPR compliant)
- Real-time analytics dashboard
- Event tracking, goals, funnels
- Lightweight (< 2KB script)
- Open source (MIT license)

**Setup**:
```bash
# 1. Prepare Umami directory on the droplet
mkdir -p /opt/umami && cd /opt/umami

# 2. Create docker-compose.yml
cat > docker-compose.yml <<EOF
version: '3.8'
services:
  postgres:
    image: postgres:15-alpine
    environment:
      POSTGRES_DB: umami
      POSTGRES_USER: umami
      POSTGRES_PASSWORD: \${DB_PASSWORD}
    volumes:
      - postgres-data:/var/lib/postgresql/data
  
  app:
    image: ghcr.io/umami-software/umami:postgresql-latest
    depends_on:
      - postgres
    ports:
      - "3000:3000"
    environment:
      DATABASE_URL: postgresql://umami:\${DB_PASSWORD}@postgres:5432/umami
      DATABASE_TYPE: postgresql
      APP_SECRET: \${APP_SECRET}
      TRACKER_SCRIPT_NAME: u.js # Obfuscate from ad blockers

volumes:
  postgres-data:
EOF

# 3. Deploy Umami
export DB_PASSWORD=$(openssl rand -hex 16)
export APP_SECRET=$(openssl rand -hex 32)
docker compose up -d

# 4. Initialize Umami
echo "Open https://analytics.apexmediation.ee (via Nginx) and log in as admin (change password)."

# 5. Add website
# Settings → Websites → Add Website
# Name: "ApexMediation Homepage"
# Domain: apexmediation.ee
# Copy tracking code

# 6. Add tracking code to website
<script defer src="https://analytics.apexmediation.ee/u.js" data-website-id="YOUR-WEBSITE-ID"></script>

# 7. Track events
umami.track('signup', { plan: 'indie' });
umami.track('sdk_download', { platform: 'ios' });

# 8. Create dashboard
# Dashboard → Custom → Add widgets (page views, unique visitors, bounce rate, top pages)
```

---

## Cost Comparison: Before vs After

### Cost Snapshot (DigitalOcean-first)
| Service | Monthly Cost | Notes |
|---------|--------------|-------|
| DigitalOcean Droplet (2 vCPU/4GB/80GB) | ~$24 | Single droplet `apex-core-1` |
| DO Managed Postgres (Basic/Dev) | ~$15 | SSL required, automated backups |
| Object Storage (MinIO self-hosted, optional Spaces offsite) | ~$0–5 | Uses droplet disk; optional Spaces for offsite |
| Misc (egress, DNS, backups) | $3–5 | Buffer for bandwidth/monitoring |
| Optional: Grafana/Prometheus | $0 | Self-host on droplet; low retention |
| Optional: Status Page (Upptime) | $0 | GitHub Pages |
| Optional: Email (Postfix) | $0 | Self-hosted, unlimited |
| **Infra Subtotal** | **$44–49** | ≤ $50 cap |

---

## Rollback Plan

### If Migration Fails
1. **Keep old infrastructure running** for 2 weeks minimum
2. **Dual-run strategy**: Route 10% traffic to new stack, 90% to old
3. **Monitoring**: Compare latency, error rates, uptime between stacks
4. **Rollback triggers**:
   - Latency >200ms (old stack avg: 50-100ms)
   - Error rate >0.5% (old stack avg: 0.1%)
   - Downtime >5 minutes in 24 hours
5. **DNS switchback**: Update DNS to point back to old infrastructure (5 min TTL for fast rollback)

### Gradual Migration
- **Week 1**: Backend API (non-critical endpoints)
- **Week 2**: Database (read replicas first)
- **Week 3**: Full backend + database cutover
- **Week 4**: Console, monitoring, marketing tools
- **Week 5**: Decommission old infrastructure

---

## Success Criteria

✅ **Cost Reduction**: <$200/month operational costs (target achieved at $133/month)
✅ **Performance**: <200ms P95 latency (comparable to old stack)
✅ **Reliability**: >99.9% uptime (measured over 30 days)
✅ **Break-even**: 2 customers @ $67/month (vs 7 in old stack)
✅ **Profit Margin**: 95%+ at 100 customers ($15K MRR - $800 costs = $14.2K profit)

---

## Automated Verification (Repo Tests)

To prevent drift between this plan and the actual repository artifacts, an automated test suite is included in the repo. It validates that key infra files and runbooks match the guidance in this document.

How to run:

```
npm run test:infra
```

What is verified (high-level):
- docker-compose.prod.yml
  - Safe local defaults for `DATABASE_URL` and `REDIS_URL` with password and private networking
  - Healthchecks for Postgres and Redis
  - Nginx port exposed via `${NGINX_PORT:-8080}` and Console gated behind the `ui` profile
- Nginx configs
  - Security headers, HSTS header present but commented (gate until A/A+)
  - HTTPS server blocks include `snippets/ssl-params.conf` and optional `/metrics` protection comment
- Evidence and DO readiness scripts
  - `scripts/ops/local_health_snapshot.sh` captures `/health` via Nginx and executes Redis verification inside the backend container
  - `scripts/ops/do_tls_snapshot.sh` saves HTTPS/TLS/HSTS evidence to a dated directory
- Docs cross-links
  - DO Readiness checklist contains Phase 9 with TLS/HSTS gating, DB TLS (`sslmode=require`), Redis isolation, and references to the TLS snapshot script
  - Production Readiness checklist includes a Post‑DO HTTPS/HSTS verification section

If a test fails, update either the docs or the corresponding artifact to bring the repo back to a consistent, production‑ready posture.

---

## Next Steps After Infrastructure Migration

1. **Marketing Infrastructure** (Week 6-7): Blog (Ghost), SEO tools, social media automation
2. **Growth Features** (Week 8-10): Customer health scoring, churn prediction, automated upsells
3. **DevOps Automation** (Week 11-12): Self-hosted CI/CD, automated backups, blue-green deployments
4. **Intelligence** (Week 13-14): Competitor monitoring, A/B testing, product analytics
5. **Launch Preparation** (Week 15-16): Security audit, performance testing, documentation finalization

**Total Time to Launch**: 16 weeks (4 months)

---

Appendix — Ops templates added in repo

- Environment templates
  - Backend: `infrastructure/production/.env.backend.example` (uses `sslmode=require` for Postgres; Redis `requirepass`)
  - Console: `infrastructure/production/.env.console.example` with `NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/api/v1`
  - Materialize real `.env` files out-of-repo or on the droplet; do not commit secrets.

- Operator runbooks (pre‑DO)
  - Backend environment reference: `docs/Internal/Deployment/BACKEND_ENVIRONMENT.md`
  - Console environment reference: `docs/Internal/Deployment/CONSOLE_ENVIRONMENT.md`
  - Local prod‑like validation: `docs/Internal/Deployment/LOCAL_PROD_VALIDATION.md`

- Nginx security snippets
  - TLS params: `infrastructure/nginx/snippets/ssl-params.conf`
  - Metrics Basic Auth: `infrastructure/nginx/snippets/metrics-basic-auth.conf` (see Section 1.4 for enablement)

- Backup script template (Postgres → S3-compatible)
  - `scripts/backup/pg_dump_s3_template.sh`
  - Usage (example for DO Spaces FRA1):
    ```bash
    export PGHOST=<do-pg-host> PGPORT=25060 PGDATABASE=ad_platform PGUSER=apex_admin PGPASSWORD=<admin-pass>
    export AWS_ACCESS_KEY_ID=<spaces-key> AWS_SECRET_ACCESS_KEY=<spaces-secret>
    export AWS_DEFAULT_REGION=eu-central-1 S3_ENDPOINT=https://fra1.digitaloceanspaces.com
    export S3_BUCKET=s3://apex-prod-backups S3_PREFIX=pg/ BACKUP_LABEL=prod
    bash scripts/backup/pg_dump_s3_template.sh
    ```
  - Retention: enforce via bucket lifecycle rules (30–90 days); script prints reminder.

---

Appendix — Services Alignment (Phase 4)

- Minimal production service set (pre‑DO and on DO):
  - backend (API) — exposed only to Nginx on the private bridge
  - console (Next.js dashboard) — served via Nginx
  - redis (self‑hosted) — private bridge only, `requirepass`, memory cap, AOF
  - nginx (reverse proxy) — HTTP on 80 locally; HTTPS split config mounted only after certs exist on DO

- Intentionally excluded from production compose unless explicitly enabled later:
  - Any experimental/auxiliary services in `services/` or elsewhere not required for core operation
  - Analytics/observability stacks beyond the basics (enable later as needed per Plan)

- Guidance for enabling additional services later:
  - Add a commented service block in `infrastructure/docker-compose.prod.yml` with private‑only networking
  - Provide an example env template under `infrastructure/production/` (do not commit secrets)
  - Ensure Nginx exposure is explicit and minimal; no public ports for internal services
  - Update DO Readiness Checklist with any new secrets/ports and verify with local prod‑like validation

> Provisioning status: As of now, the DigitalOcean account is not yet provisioned. This plan and the repo are aligned to be “DO‑ready,” and CI is configured to avoid accidental deploys until DO secrets exist. See “0. Readiness & Enablement” below.

### 0. Readiness & Enablement (Before Creating DO Resources)

- Repo/CI safe mode
  - The deploy workflow `.github/workflows/deploy-do.yml` runs only on manual dispatch and will no‑op the droplet deploy steps when DO secrets are missing.
  - Build/push to GHCR still works so images can be validated ahead of DO provisioning.
- Pre‑provisioning checklist (see `docs/Internal/Infrastructure/DO_READINESS_CHECKLIST.md` for a step‑by‑step list):
  - Register DO account; set billing.
  - Decide domains and DNS provider (Cloudflare/DNS host). Keep A/AAAA records planned but not live.
  - Generate a dedicated deploy SSH keypair for the droplet.
  - Prepare GitHub repo secrets you’ll need later (do not add yet): `DROPLET_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`.
  - Decide FRA1 names: droplet `apex-core-1`, PG cluster name, Spaces bucket name.
