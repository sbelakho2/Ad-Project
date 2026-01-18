# Rate Limiting & DDoS Protection

**Last Updated:** January 2026  
**Owner:** Infrastructure Team

---

## Overview

This document describes the rate limiting and DDoS protection strategies implemented across the Apex Mediation Platform.

---

## 1. Rate Limiting Architecture

### 1.1 Multi-Layer Approach

```
┌─────────────────────────────────────────────────────────────┐
│                     LAYER 1: Edge (Caddy)                  │
│         • Connection limits • Basic rate limiting          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                  LAYER 2: Application (Express)            │
│     • Redis-backed rate limiter • Per-route limits         │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                  LAYER 3: Service (Business Logic)         │
│      • Per-tenant limits • Per-operation limits            │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Backend API Rate Limits

### 2.1 Default Limits

| Endpoint Category | Window | Max Requests | Identifier |
|-------------------|--------|--------------|------------|
| Public API | 1 min | 100 | IP address |
| Authenticated API | 1 min | 300 | User ID |
| Auth endpoints | 1 min | 10 | IP address |
| Webhook endpoints | 1 min | 1000 | Source IP |

### 2.2 Implementation

**File:** `backend/src/middleware/redisRateLimiter.ts`

```typescript
// Example configuration
export const apiRateLimiter = createRateLimiter({
  windowMs: 60 * 1000,        // 1 minute
  max: 100,                    // 100 requests per window
  keyGenerator: (req) => req.ip,
  handler: (req, res) => {
    res.status(429).json({
      error: 'Too many requests',
      retryAfter: res.getHeader('Retry-After'),
    });
  },
});

export const authRateLimiter = createRateLimiter({
  windowMs: 60 * 1000,
  max: 10,                     // Stricter for auth
  keyGenerator: (req) => req.ip,
});
```

### 2.3 Rate Limit Headers

All responses include standard rate limit headers:

```
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 87
X-RateLimit-Reset: 1642531200
Retry-After: 45  (only on 429 responses)
```

---

## 3. SSAI-Specific Rate Limits

### 3.1 Session Limits

**File:** `backend/src/services/ssai/session/SecurityHandler.ts`

| Limit Type | Default | Configurable |
|------------|---------|--------------|
| Sessions per tenant | 10,000 | Yes (env) |
| Sessions per IP | 100 | Yes |
| Requests per session | 1000/min | Yes |

### 3.2 Implementation

```typescript
export interface RateLimitConfig {
  maxRequests: number;
  windowMs: number;
  keyPrefix: string;
}

checkRateLimit(
  identifier: string,
  config: RateLimitConfig
): RateLimitResult {
  // Sliding window rate limiting
  // Returns: { allowed: boolean, remaining: number, resetAt: number }
}
```

---

## 4. RTB Auction Rate Limits

### 4.1 Per-Publisher Limits

| Publisher Tier | Auctions/sec | Burst Capacity |
|----------------|--------------|----------------|
| Free | 10 | 20 |
| Growth | 100 | 200 |
| Enterprise | Unlimited | N/A |

### 4.2 Circuit Breaker

If a publisher exceeds limits consistently:

1. **Soft limit exceeded** → Warning logged, metrics incremented
2. **Hard limit exceeded** → 429 response, temporary cooldown
3. **Repeated violations** → Circuit breaker opens for 5 minutes

---

## 5. DDoS Protection

### 5.1 Caddy Layer

**File:** `infrastructure/caddy/Caddyfile`

```caddyfile
{
    servers {
        max_header_size 16KB
    }
}

api.apexmediation.ee {
    # Connection limits
    rate_limit {
        zone api_zone {
            key {remote_host}
            events 100
            window 1m
        }
    }
    
    # Request size limits
    request_body {
        max_size 1MB
    }
}
```

### 5.2 Recommended: Cloudflare Integration

For production deployments, we recommend adding Cloudflare in front of Caddy:

| Protection | Cloudflare Feature |
|------------|-------------------|
| L3/L4 DDoS | Always-on |
| L7 DDoS | WAF rules |
| Bot mitigation | Bot Management |
| Rate limiting | Configurable rules |

**Setup Guide:** See `docs/Internal/Infrastructure/CLOUDFLARE_SETUP.md`

### 5.3 Hetzner Firewall

**File:** `infrastructure/hetzner/setup-app-server.sh`

```bash
# UFW rules for DDoS mitigation
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp    # SSH
ufw allow 80/tcp    # HTTP (redirect to HTTPS)
ufw allow 443/tcp   # HTTPS

# Connection tracking limits
iptables -A INPUT -p tcp --syn -m connlimit --connlimit-above 50 -j DROP
```

---

## 6. Monitoring & Alerting

### 6.1 Rate Limit Metrics

**Prometheus metrics exposed:**

```
# Rate limit hits
rate_limit_hits_total{endpoint, status}

# Current rate limit usage
rate_limit_current{endpoint, identifier}

# Circuit breaker status
circuit_breaker_state{service, state}
```

### 6.2 Alerts

**File:** `monitoring/alerts.yml`

```yaml
- alert: HighRateLimitHits
  expr: rate(rate_limit_hits_total{status="blocked"}[5m]) > 100
  for: 5m
  labels:
    severity: warning
  annotations:
    summary: "High rate limit blocks detected"
    description: "{{ $value }} requests/sec being blocked"

- alert: PotentialDDoS
  expr: rate(http_requests_total[1m]) > 10000
  for: 2m
  labels:
    severity: critical
  annotations:
    summary: "Potential DDoS attack detected"
```

---

## 7. Configuration Reference

### 7.1 Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RATE_LIMIT_WINDOW_MS` | 60000 | Rate limit window |
| `RATE_LIMIT_MAX_REQUESTS` | 100 | Max requests per window |
| `RATE_LIMIT_AUTH_MAX` | 10 | Auth endpoint limit |
| `SSAI_MAX_SESSIONS_PER_TENANT` | 10000 | SSAI session limit |

### 7.2 Redis Configuration

Rate limiting uses Redis for distributed state:

```
REDIS_URL=redis://localhost:6379
REDIS_RATE_LIMIT_PREFIX=rl:
REDIS_RATE_LIMIT_TTL=120  # 2x window for safety
```

---

## 8. Testing Rate Limits

### 8.1 Manual Testing

```bash
# Test rate limit (should get 429 after 10 requests)
for i in {1..15}; do
  curl -s -o /dev/null -w "%{http_code}\n" \
    -X POST https://api.apexmediation.ee/api/v1/auth/login \
    -H "Content-Type: application/json" \
    -d '{"email":"test@example.com","password":"wrong"}'
done
```

### 8.2 Load Testing

```bash
# Use k6 for load testing
k6 run scripts/load-tests/rate-limit-test.js
```

---

## 9. Incident Response

If a DDoS attack is detected:

1. **Immediate:** Check Grafana dashboard for traffic patterns
2. **Triage:** Identify attack vector (L3/L4 vs L7)
3. **Mitigate:** 
   - Enable Cloudflare "Under Attack" mode
   - Adjust rate limits dynamically
   - Block malicious IPs at firewall
4. **Communicate:** Update status page if service degraded
5. **Post-mortem:** Document attack and improve defenses

See: `docs/runbooks/ddos-incident.md`
