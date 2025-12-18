# Rate Limiting Architecture

**Last Updated:** December 17, 2025  
**Target Audience:** Backend Engineers, DevOps, Security Team

---

## Overview

ApexMediation implements **tiered rate limiting** that scales with customer subscription tier. The system uses Redis-backed sliding window counters with fail-open behavior when Redis is unavailable.

### Design Goals

1. **Fair Usage**: Ensure all customers get appropriate API access for their tier
2. **Security**: Protect authentication endpoints from brute-force attacks
3. **High Availability**: Fail open to avoid service disruption
4. **Observability**: Standard rate limit headers for client debugging
5. **Flexibility**: Environment variable overrides for tuning without code changes

---

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────────┐
│                     Express Application                      │
├─────────────────────────────────────────────────────────────┤
│  Global Rate Limiter (express-rate-limit, in-memory fallback)│
├─────────────────────────────────────────────────────────────┤
│  Tiered Rate Limiter (Redis-backed, tier-aware)             │
│  ├── authTieredRateLimiter                                  │
│  ├── dashboardRateLimiter                                   │
│  ├── analyticsRateLimiter                                   │
│  ├── transparencyRateLimiter                                │
│  └── rtbRateLimiter                                         │
├─────────────────────────────────────────────────────────────┤
│  Redis (sliding window counters)                            │
│  Key pattern: rl:{category}:{tier}:{publisherId}:{ip}       │
└─────────────────────────────────────────────────────────────┘
```

### Key Files

| File | Purpose |
|------|---------|
| `backend/src/config/rateLimits.ts` | Tier configurations, endpoint limits, exempt routes |
| `backend/src/middleware/tieredRateLimiter.ts` | Redis-backed middleware implementation |
| `backend/src/index.ts` | Rate limiter application to routes |

---

## Rate Limit Tiers

Limits scale ~5x between tiers to match expected usage patterns.

### Global API Limits

| Tier | Global | Auth | Dashboard | Analytics | Transparency | RTB |
|------|--------|------|-----------|-----------|--------------|-----|
| **Starter** | 1,000/min | 20/min | 300/min | 5,000/min | 120/min | 1,000/min |
| **Growth** | 5,000/min | 50/min | 1,000/min | 25,000/min | 500/min | 5,000/min |
| **Scale** | 20,000/min | 100/min | 3,000/min | 100,000/min | 2,000/min | 20,000/min |
| **Enterprise** | 100,000/min | 200/min | 10,000/min | 500,000/min | 10,000/min | 100,000/min |

### Unauthenticated Limits

Anonymous/public requests receive restrictive limits:

| Category | Limit | Window |
|----------|-------|--------|
| Global | 60/min | 1 minute |
| Auth | 10/min | 1 minute |
| Dashboard | 30/min | 1 minute |
| Analytics | 100/min | 1 minute |
| Transparency | 30/min | 1 minute |
| RTB | 0 (blocked) | - |

### Endpoint-Specific Limits

Security-sensitive endpoints have stricter per-IP limits:

| Endpoint | Limit | Window | Reason |
|----------|-------|--------|--------|
| `POST /auth/login` | 5 | 1 min | Brute-force protection |
| `POST /auth/register` | 3 | 5 min | Spam prevention |
| `POST /auth/forgot-password` | 3 | 5 min | Email bombing prevention |
| `POST /auth/verify-2fa` | 5 | 1 min | 2FA brute-force protection |
| `POST /auth/reset-password` | 3 | 5 min | Token guessing prevention |

---

## Exempt Routes

The following routes bypass rate limiting:

```typescript
export const RATE_LIMIT_EXEMPT_ROUTES = [
  '/health',
  '/ready', 
  '/metrics',
  '/api/v1/status.json',
  '/api/v1/slo.json',
  '/openapi.json',
  '/docs',
];
```

**Rationale:**
- Health/ready endpoints must be available for load balancers
- Metrics must be scrapable by Prometheus
- Status/SLO endpoints power the public status page (cached)

---

## Implementation Details

### Redis Key Pattern

```
rl:{category}:{tier}:{publisherId}:{ip}
```

Example: `rl:auth:starter:pub_abc123:192.168.1.1`

### Sliding Window Algorithm

Uses simple increment/expire pattern (fixed window with TTL):

```typescript
async function checkRateLimit(key: string, limit: number, windowMs: number) {
  const windowSeconds = Math.ceil(windowMs / 1000);
  
  // Increment counter
  const current = await redis.incr(key);
  
  // Set expiry on first hit
  if (current === 1) {
    await redis.expire(key, windowSeconds);
  }
  
  return { allowed: current <= limit, current, limit };
}
```

### Fail-Open Behavior

If Redis is unavailable, requests are allowed:

```typescript
if (!redis.isReady()) {
  return { allowed: true, current: 0, limit, resetInSeconds: 0 };
}
```

**Rationale:** Service availability > rate limiting during Redis outages.

---

## Response Headers

All rate-limited responses include standard headers:

| Header | Description | Example |
|--------|-------------|---------|
| `X-RateLimit-Limit` | Max requests in window | `1000` |
| `X-RateLimit-Remaining` | Requests remaining | `847` |
| `X-RateLimit-Reset` | Unix timestamp of reset | `1734456000` |
| `Retry-After` | Seconds until retry (429 only) | `42` |

### 429 Response Body

```json
{
  "success": false,
  "error": "Too many requests",
  "message": "Rate limit exceeded. Please try again later or upgrade your plan for higher limits.",
  "retryAfter": 42,
  "upgradeUrl": "/pricing"
}
```

---

## Environment Variables

Override defaults without code changes:

| Variable | Default | Description |
|----------|---------|-------------|
| `RATE_LIMIT_WINDOW_MS` | `60000` | Global window (ms) |
| `RATE_LIMIT_MAX_REQUESTS` | `1000` | Global max requests |
| `RATE_LIMIT_GLOBAL_MAX` | (tier-based) | Override global max |
| `RATE_LIMIT_AUTH_MAX` | (tier-based) | Override auth max |
| `RATE_LIMIT_ANALYTICS_MAX` | (tier-based) | Override analytics max |

---

## Industry Benchmarks

Our limits are based on industry standards:

| Provider | Rate Limit | Notes |
|----------|------------|-------|
| **Stripe** | 100 req/sec (6,000/min) | Per-endpoint limits of 25 req/sec |
| **GitHub** | 5,000/hour (83/min) | Authenticated; 60/hour unauthenticated |
| **Twitter/X Pro** | 900/15min (60/min) | Enterprise gets custom limits |
| **AWS API Gateway** | 10,000/sec burst | 5,000/sec sustained |

**Previous ApexMediation limits** (before Dec 2025):
- Global: 100 req/15min = **6.7 req/min** ❌ (way too low)
- Auth: 50 req/15min = **3.3 req/min** ❌

---

## Monitoring

### Prometheus Metrics

Rate limit events are logged with structured data:

```typescript
logger.warn('Rate limit exceeded', {
  category: 'auth',
  tier: 'starter',
  publisherId: 'pub_abc123',
  ip: '192.168.1.1',
  current: 21,
  limit: 20,
  path: '/api/v1/auth/login',
});
```

### Alerting

Consider alerting on:
- High rate of 429 responses (may indicate attack or misconfigured client)
- Rate limit exhaustion by tier (may indicate need for upgrade)

---

## Troubleshooting

### "429 Too Many Requests" errors

1. Check customer tier matches expected limits
2. Verify request is not hitting endpoint-specific limits
3. Check if Redis is available (fail-open should prevent this)
4. Review rate limit headers in response

### Rate limiting not working

1. Verify Redis is connected: `redis.isReady()`
2. Check route is not in `RATE_LIMIT_EXEMPT_ROUTES`
3. Verify middleware order in `index.ts`

### Customer needs higher limits

1. Check current tier in database
2. Upgrade tier or contact sales for Enterprise custom limits
3. Environment variables can temporarily override for testing

---

## Changelog

| Date | Change |
|------|--------|
| Dec 17, 2025 | Implemented tiered rate limiting with Redis backing |
| Dec 17, 2025 | Increased limits 150x (100/15min → 1000/min for Starter) |
| Dec 17, 2025 | Added tier-aware middleware and configuration |

---

**End of Document**
