# Sales Automation Operations Runbook

**Last Updated:** January 2026  
**Version:** 2.6  
**Status:** Production

---

## ⚠️ CRITICAL Architecture Note

**ALL sales automation work runs on the Test Server** (203.0.113.11:4001).  
**The App Server control-plane is strictly a proxy client.**

| Server | IP | Role | Services |
|--------|-----|------|----------|
| **Test Server** | 203.0.113.11 | **Execution Server** | sales-api (PM2:4001), edge-worker (3500), inbound-smtp (2525) |
| **App Server** | 203.0.113.10 | **Proxy Client** | control-plane (4000), sales-api-tunnel, edge-tunnel |
| **DB Server** | 203.0.113.12 | Database | PostgreSQL (via App Server PgBouncer only) |

---

## Table of Contents

1. [Health Checks](#health-checks)
2. [Starting/Stopping Services](#startingstopping-services)
3. [Viewing Logs](#viewing-logs)
4. [SSH Tunnels](#ssh-tunnels)
5. [Database Access](#database-access)
6. [Common Issues](#common-issues)
7. [Emergency Procedures](#emergency-procedures)

---

## Health Checks

### Test Server (Sales Automation)

```bash
# Check all services status
./scripts/ops/hetzner_ssh.sh test -- "pm2 status && sudo systemctl status db-tunnel.service inbound-smtp.service edge-worker.service"

# Quick health check via API
./scripts/ops/hetzner_ssh.sh test -- "curl -s http://localhost:4001/api/autonomous/health | jq"

# Check PM2 process details
./scripts/ops/hetzner_ssh.sh test -- "pm2 describe sales-api"
```

### App Server (Proxy)

```bash
# Check control-plane status
./scripts/ops/hetzner_ssh.sh app -- "pm2 status"

# Check SSH tunnels
./scripts/ops/hetzner_ssh.sh app -- "sudo systemctl status sales-api-tunnel.service edge-tunnel.service"

# Verify proxy connectivity (should return Test Server health)
./scripts/ops/hetzner_ssh.sh app -- "curl -s http://localhost:4001/api/autonomous/health | jq"
```

### End-to-End Test

```bash
# Test from App Server control-plane through to Test Server
./scripts/ops/hetzner_ssh.sh app -- "curl -s -X POST http://localhost:4000/api/autonomous/compose \
  -H 'Content-Type: application/json' \
  -H 'X-API-Key: edge-intel-test-1769358396' \
  -d '{\"contact\":{\"name\":\"Test\",\"company_name\":\"TestCorp\"}}' | jq '.message'"
```

---

## Starting/Stopping Services

### Test Server - Sales API

```bash
# Start
./scripts/ops/hetzner_ssh.sh test -- "pm2 start sales-api"

# Stop
./scripts/ops/hetzner_ssh.sh test -- "pm2 stop sales-api"

# Restart
./scripts/ops/hetzner_ssh.sh test -- "pm2 restart sales-api"

# Reload (zero-downtime)
./scripts/ops/hetzner_ssh.sh test -- "pm2 reload sales-api"
```

### Test Server - DB Tunnel

```bash
# Start
./scripts/ops/hetzner_ssh.sh test -- "sudo systemctl start db-tunnel.service"

# Stop
./scripts/ops/hetzner_ssh.sh test -- "sudo systemctl stop db-tunnel.service"

# Restart
./scripts/ops/hetzner_ssh.sh test -- "sudo systemctl restart db-tunnel.service"
```

### App Server - SSH Tunnels

```bash
# Restart sales-api tunnel
./scripts/ops/hetzner_ssh.sh app -- "sudo systemctl restart sales-api-tunnel.service"

# Restart edge tunnel
./scripts/ops/hetzner_ssh.sh app -- "sudo systemctl restart edge-tunnel.service"
```

### App Server - Control Plane

```bash
# Restart control-plane
./scripts/ops/hetzner_ssh.sh app -- "pm2 restart control-plane"

# If port 4000 is stuck:
./scripts/ops/hetzner_ssh.sh app -- "sudo fuser -k 4000/tcp && pm2 restart control-plane"
```

---

## Viewing Logs

### Test Server - Sales API Logs

```bash
# Last 100 lines
./scripts/ops/hetzner_ssh.sh test -- "pm2 logs sales-api --lines 100"

# Follow live logs
./scripts/ops/hetzner_ssh.sh test -- "pm2 logs sales-api --follow"

# Errors only
./scripts/ops/hetzner_ssh.sh test -- "pm2 logs sales-api --err --lines 100"
```

### Test Server - DB Tunnel Logs

```bash
./scripts/ops/hetzner_ssh.sh test -- "sudo journalctl -u db-tunnel.service -n 50"
```

### App Server - Control Plane Logs

```bash
./scripts/ops/hetzner_ssh.sh app -- "pm2 logs control-plane --lines 100"
```

### App Server - SSH Tunnel Logs

```bash
./scripts/ops/hetzner_ssh.sh app -- "sudo journalctl -u sales-api-tunnel.service -n 50"
./scripts/ops/hetzner_ssh.sh app -- "sudo journalctl -u edge-tunnel.service -n 50"
```

---

## SSH Tunnels

### Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         APP SERVER (203.0.113.10)                        │
├──────────────────────────────────────────────────────────────────────────┤
│  sales-api-tunnel.service                                                │
│    localhost:4001 ───SSH──────────────────────> Test Server:4001         │
│                                                                          │
│  edge-tunnel.service                                                     │
│    localhost:3500 ───SSH──────────────────────> Test Server:3500         │
└──────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────┐
│                       TEST SERVER (203.0.113.11)                         │
├──────────────────────────────────────────────────────────────────────────┤
│  db-tunnel.service                                                       │
│    localhost:6432 ───SSH──────────────────────> App Server:6432          │
│                                                 (PgBouncer ─> DB:5432)   │
└──────────────────────────────────────────────────────────────────────────┘
```

### Troubleshooting Tunnels

```bash
# Check if tunnel port is listening (App Server)
./scripts/ops/hetzner_ssh.sh app -- "ss -tlnp | grep -E '4001|3500'"

# Check if tunnel port is listening (Test Server)
./scripts/ops/hetzner_ssh.sh test -- "ss -tlnp | grep 6432"

# Test tunnel connectivity
./scripts/ops/hetzner_ssh.sh app -- "nc -zv localhost 4001"
./scripts/ops/hetzner_ssh.sh test -- "nc -zv localhost 6432"
```

---

## Database Access

### From Test Server (via db-tunnel)

```bash
# Quick connection test
./scripts/ops/hetzner_ssh.sh test -- "psql 'postgres://adproject:***@localhost:6432/adproject?sslmode=disable' -c 'SELECT 1'"

# Interactive psql
./scripts/ops/hetzner_ssh.sh test -- "psql 'postgres://adproject:***@localhost:6432/adproject?sslmode=disable'"
```

### Sales Automation Tables

```sql
-- Check recent leads
SELECT id, email, company_name, score, status, created_at 
FROM sales.leads 
ORDER BY created_at DESC 
LIMIT 10;

-- Check campaign stats
SELECT variant_id, opens, clicks, replies, conversions 
FROM sales.thompson_stats 
ORDER BY conversions DESC;

-- Check email queue
SELECT id, to_email, subject, status, created_at 
FROM sales.email_queue 
WHERE status != 'sent' 
ORDER BY created_at DESC 
LIMIT 20;
```

---

## Common Issues

### Issue: Control Plane returns 502 on /api/autonomous/*

**Cause:** SSH tunnel to Test Server is down.

**Fix:**
```bash
# Restart tunnel
./scripts/ops/hetzner_ssh.sh app -- "sudo systemctl restart sales-api-tunnel.service"

# Verify
./scripts/ops/hetzner_ssh.sh app -- "curl -s http://localhost:4001/api/autonomous/health"
```

### Issue: sales-api can't connect to database

**Cause:** db-tunnel.service on Test Server is down.

**Fix:**
```bash
# Restart db-tunnel
./scripts/ops/hetzner_ssh.sh test -- "sudo systemctl restart db-tunnel.service"

# Verify
./scripts/ops/hetzner_ssh.sh test -- "psql 'postgres://adproject:***@localhost:6432/adproject?sslmode=disable' -c 'SELECT 1'"

# Restart sales-api
./scripts/ops/hetzner_ssh.sh test -- "pm2 restart sales-api"
```

### Issue: Port 4000 EADDRINUSE on App Server

**Cause:** Old control-plane process didn't exit cleanly.

**Fix:**
```bash
./scripts/ops/hetzner_ssh.sh app -- "sudo fuser -k 4000/tcp && pm2 restart control-plane"
```

### Issue: Emails not being sent

**Check queue:**
```bash
./scripts/ops/hetzner_ssh.sh test -- "psql 'postgres://adproject:***@localhost:6432/adproject?sslmode=disable' \
  -c \"SELECT status, COUNT(*) FROM sales.email_queue GROUP BY status;\""
```

**Check SMTP config:**
```bash
./scripts/ops/hetzner_ssh.sh test -- "grep -E 'SMTP|RESEND' /opt/adproject/control-plane/.env | head -5"
```

---

## Emergency Procedures

### Full Service Restart

```bash
# 1. Test Server: Restart all services
./scripts/ops/hetzner_ssh.sh test -- "sudo systemctl restart db-tunnel.service && pm2 restart all"

# 2. App Server: Restart tunnels and control-plane
./scripts/ops/hetzner_ssh.sh app -- "sudo systemctl restart sales-api-tunnel.service edge-tunnel.service && pm2 restart control-plane"

# 3. Verify
./scripts/ops/hetzner_ssh.sh app -- "curl -s http://localhost:4000/api/autonomous/health | jq"
```

### Rollback to Previous Code

```bash
# On Test Server
./scripts/ops/hetzner_ssh.sh test -- "cd /opt/adproject/control-plane && git log --oneline -5"
./scripts/ops/hetzner_ssh.sh test -- "cd /opt/adproject/control-plane && git checkout <previous-commit>"
./scripts/ops/hetzner_ssh.sh test -- "pm2 restart sales-api"
```

### Disable Sales Automation (Emergency)

```bash
# Stop sales-api on Test Server
./scripts/ops/hetzner_ssh.sh test -- "pm2 stop sales-api"

# Control Plane will return 502 on /api/autonomous/* until restarted
```

---

## Related Documentation

- [AUTONOMOUS_SALES_V2.md](../Internal/Sales/AUTONOMOUS_SALES_V2.md) - Full architecture and implementation guide
- [AUTONOMOUS_SALES_V2_QUICK_REF.md](../Internal/Sales/AUTONOMOUS_SALES_V2_QUICK_REF.md) - Quick reference
- [AUTONOMOUS_SALES_API.md](../Developer-Facing/AUTONOMOUS_SALES_API.md) - API reference
- [HETZNER_PRODUCTION_CHECKLIST.md](../Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md) - Production checklist
- [infrastructure/hetzner/README.md](../../infrastructure/hetzner/README.md) - Infrastructure overview
