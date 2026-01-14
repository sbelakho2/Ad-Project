# ApexMediation Infrastructure

Infrastructure as Code (IaC) and deployment configurations for the ApexMediation platform.

## 🏗️ Overview

The platform is designed for ultra-lean operations, targeting low monthly costs while maintaining high availability and global performance.

## 🛠️ Stack

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
