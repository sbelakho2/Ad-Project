# Rollback Runbook

**Last Updated:** 2025-01-06  
**Owner:** DevOps  
**Approval Required:** On-call lead for production rollback

---

## Table of Contents

1. [Quick Reference](#quick-reference)
2. [Pre-Rollback Checklist](#pre-rollback-checklist)
3. [Rollback Procedures](#rollback-procedures)
4. [Database Rollback](#database-rollback)
5. [Post-Rollback Verification](#post-rollback-verification)
6. [Escalation Matrix](#escalation-matrix)

---

## Quick Reference

### SSH Access (Hetzner Production)
```bash
# App Server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# DB Server  
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35
```

### Current Services (Hetzner)
| Service | Port | Server | Notes |
|---------|------|--------|-------|
| Backend | 8080 | 46.62.252.96 | Node.js API |
| Console | 3000 | 46.62.252.96 | Next.js dashboard |
| Grafana | 3001 | 46.62.252.96 | Monitoring dashboards |
| Prometheus | 9090 | 46.62.252.96 | Metrics collection |
| Alertmanager | 9093 | 46.62.252.96 | Alert routing |
| PostgreSQL | 5432 | 77.42.44.35 | Database |
| Redis | 6379 | 46.62.252.96 | Cache (container) |

### Health Endpoints
```bash
# Quick health check (Hetzner)
curl -s http://46.62.252.96:8080/health | jq .
curl -s http://46.62.252.96:8080/ready | jq .
curl -s http://46.62.252.96:3000 | head -1

# Prometheus targets
curl -s http://46.62.252.96:9090/api/v1/targets | jq '.data.activeTargets | length'
```

### Monitoring Links
- **Grafana:** http://46.62.252.96:3001 (credentials in secret store)
- **Prometheus:** http://46.62.252.96:9090
- **Alertmanager:** http://46.62.252.96:9093

---

## Pre-Rollback Checklist

Before initiating rollback:

- [ ] **Identify the issue** - What broke? Is rollback the right solution?
- [ ] **Check monitoring** - Review Grafana dashboards, Prometheus alerts
- [ ] **Document the current state** - Take screenshots, save logs
- [ ] **Notify stakeholders** - Alert team in Slack/on-call channel
- [ ] **Verify rollback target** - Which version to rollback to?
- [ ] **Check for database migrations** - Were migrations run? (See [Database Rollback](#database-rollback))
- [ ] **Backup current state** - Export critical data if needed

---

## Rollback Procedures

### Option 1: Quick Container Rollback (No Code Changes)

If issue is container-related (crash, resource exhaustion):

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96

# Restart affected service
cd Ad-Project
docker compose restart backend   # or: console, redis

# Verify health
curl -s http://localhost:8080/health | jq .
```

### Option 2: Git Rollback (Code Changes)

If issue is in recent code changes:

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd Ad-Project

# View recent commits
git log --oneline -10

# Identify last known good commit
# Example: Rollback to commit abc1234

# Rollback code
git fetch origin
git reset --hard <good-commit-hash>

# Rebuild and restart services
docker compose stop backend
docker compose rm -f backend
docker compose up --build -d backend

# Verify
curl -s http://localhost:8080/health | jq .
```

### Option 3: Docker Image Rollback

If you have tagged releases:

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd Ad-Project

# Stop current container
docker compose stop backend

# Edit docker-compose.yml to use specific image tag
# Or pull previous image:
docker pull ghcr.io/sbelakho2/ad-project-backend:v1.2.3

# Update and restart
docker compose up -d backend
```

### Option 4: Full Stack Rollback

For complete environment issues:

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd Ad-Project

# Stop all services
docker compose down

# Reset to known good state
git fetch origin
git reset --hard <good-commit-hash>

# Rebuild everything
docker compose up --build -d

# Wait for health
sleep 30
curl -s http://localhost:8080/health | jq .
curl -s http://localhost:3000 | head -1
```

---

## Database Rollback

⚠️ **WARNING:** Database rollbacks can cause data loss. Use extreme caution.

### Check Migration Status

```bash
# SSH to db server
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35

# List applied migrations
psql -U postgres -d adproject -c \
  "SELECT * FROM migrations ORDER BY id DESC LIMIT 10;"
```

### Rollback Last Migration

```bash
# SSH to app server
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
cd Ad-Project

# Run migration rollback (if supported)
docker exec ad-project-backend-1 npm run migrate:down

# Verify on DB server
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35
psql -U postgres -d adproject -c \
  "SELECT * FROM migrations ORDER BY id DESC LIMIT 5;"
```

### Full Database Restore

If you have a backup:

```bash
# SSH to db server
ssh -i ~/.ssh/hetzner-db-mac root@77.42.44.35

# Restore from backup (after stopping backend on app server)
psql -U postgres -d adproject < /path/to/backup.sql
```

---

## Post-Rollback Verification

After any rollback, verify:

### 1. Health Checks
```bash
# API health
curl -s http://46.62.252.96:8080/health | jq .
# Expected: {"status":"healthy","services":{"postgres":"up","redis":"up","queues":"up"}}

curl -s http://46.62.252.96:8080/ready | jq .
# Expected: {"status":"ready","database":{"connected":true}}

# Console health
curl -s -o /dev/null -w "%{http_code}" http://46.62.252.96:3000
# Expected: 200

# Monitoring health
curl -s http://46.62.252.96:9090/api/v1/targets | jq '.data.activeTargets | length'
# Expected: 7
```

### 2. Core Functionality
```bash
# Authentication
curl -s -X POST http://46.62.252.96:8080/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@hetzner-test.com","password":"Admin123!"}' | jq .success
# Expected: true

# RTB endpoint
curl -s -X POST http://46.62.252.96:8080/api/v1/rtb/bid \
  -H "Content-Type: application/json" \
  -H "X-SDK-Key: ak_test_..." \
  -d '{"placementId":"test","adType":"banner"}' | jq .
# Expected: bid response
```

### 3. Container Status
```bash
docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
# All containers should show "Up" and "healthy"
```

### 4. Logs Check
```bash
# Check for errors in last 5 minutes
docker logs --since 5m ad-project-backend-1 2>&1 | grep -i error | tail -20
```

### 5. Monitoring Check
```bash
# Check Grafana (in browser)
open http://46.62.252.96:3001

# Check Prometheus targets
curl -s http://46.62.252.96:9090/api/v1/targets | jq '.data.activeTargets[] | {job: .labels.job, health: .health}'
```

---

## Escalation Matrix

| Severity | Issue | Action | Contact |
|----------|-------|--------|---------|
| P1 | Complete outage | Immediate rollback, page on-call | On-call lead |
| P2 | Major feature broken | Rollback within 1 hour | Team lead |
| P3 | Minor issue | Fix forward preferred | Developer |
| P4 | Cosmetic | No rollback needed | Backlog |

### Communication Template

```
🚨 INCIDENT: [Brief description]

Status: INVESTIGATING / MITIGATING / RESOLVED
Impact: [What's affected]
Timeline:
- HH:MM - Issue detected
- HH:MM - Rollback initiated
- HH:MM - Service restored

Next update: HH:MM
```

---

## Appendix: Useful Commands

### View Logs
```bash
docker logs -f ad-project-backend-1 --tail 100
docker logs -f ad-project-console-1 --tail 100
```

### Resource Usage
```bash
docker stats --no-stream
```

### Database Query
```bash
docker exec ad-project-postgres-1 psql -U postgres -d apex -c "SELECT COUNT(*) FROM users;"
```

### Redis Status
```bash
docker exec ad-project-redis-1 redis-cli -a changeme INFO | grep used_memory_human
```

### Git History
```bash
git log --oneline --graph --all -20
```
