# Reverse Proxy Configuration Guide

**Location**: `docs/Internal/Infrastructure/`  
**Purpose**: Document Express trust proxy settings, TLS termination, and security middleware behavior behind reverse proxies

---

## Overview

The ApexMediation backend API is designed to run behind a reverse proxy (nginx, load balancer, CDN) in production. This document covers proper configuration for:

- Trust proxy settings
- TLS termination
- Rate limiting behavior
- CSRF protection
- IP address resolution

---

## Trust Proxy Configuration

### What is Trust Proxy?

When Express runs behind a reverse proxy, the actual client IP and protocol information is passed via `X-Forwarded-*` headers. By default, Express doesn't trust these headers for security reasons.

### Enabling Trust Proxy

**Environment Variable**:
```bash
TRUST_PROXY=true
```

**Effect** (in `backend/src/index.ts`):
```typescript
if (env.TRUST_PROXY) {
  app.set('trust proxy', 1);
  logger.info('Trust proxy enabled - respecting X-Forwarded-* headers');
}
```

### Trust Proxy Values

Express supports multiple trust proxy values:

| Value | Meaning |
|-------|---------|
| `false` | Disabled (default) - don't trust any proxy |
| `true` / `1` | Trust the first hop (single proxy) |
| `2`, `3`, etc. | Trust N hops from front-facing proxy |
| `'loopback'` | Trust localhost/loopback addresses |
| `'10.0.0.0/8'` | Trust specific CIDR |
| Custom function | Advanced trust logic |

**Our Default**: `1` (single proxy hop)

**When to Use**:
- ✅ Behind Caddy reverse proxy (Hetzner)
- ✅ Behind nginx reverse proxy
- ✅ Behind AWS ALB/ELB
- ✅ Behind Cloudflare
- ❌ Development on localhost (use `false`)

---

## TLS Termination

### Architecture

```
Internet → [TLS] → Load Balancer → [HTTP] → Backend API
```

**TLS is terminated at the reverse proxy layer**, not in the Node.js application.

### Configuration Points

#### 1. Reverse Proxy (nginx example)

```nginx
server {
    listen 443 ssl http2;
    server_name api.apexmediation.ee;

    # TLS certificates
    ssl_certificate /etc/letsencrypt/live/api.apexmediation.ee/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.apexmediation.ee/privkey.pem;
    
    # Strong TLS configuration
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers 'ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256';
    ssl_prefer_server_ciphers off;

    # Forward headers
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;

    location / {
      proxy_pass http://127.0.0.1:8080;
    }
}
```

#### 2. Backend API (no TLS needed)

The backend listens on HTTP (port 8080 in Hetzner production) and trusts X-Forwarded-Proto for protocol detection.

#### 3. CORS Configuration

Update `.env` for production:
```bash
CORS_ORIGIN=https://console.apexmediation.ee
# Or multiple origins
CORS_ALLOWLIST=https://console.apexmediation.ee,https://www.apexmediation.ee
```

---

## Security Middleware Behavior

### Rate Limiting

**Middleware**: `express-rate-limit` + Redis store

**IP Resolution**:
- **With `trust proxy`**: Uses `X-Forwarded-For` header (real client IP)
- **Without**: Uses connection IP (proxy IP) - **ineffective**

**Configuration** (`backend/src/index.ts`):
```typescript
const limiter = rateLimit({
  windowMs: env.RATE_LIMIT_WINDOW_MS, // 15 minutes default
  max: env.RATE_LIMIT_MAX_REQUESTS,    // 100 requests default
  message: 'Too many requests from this IP, please try again later.',
  // Trust proxy is inherited from app.set('trust proxy', 1)
});
```

**Why Trust Proxy Matters**:
```
Without trust proxy:
  Client A → Proxy (1.2.3.4) → Backend sees 1.2.3.4
  Client B → Proxy (1.2.3.4) → Backend sees 1.2.3.4
  ❌ Both clients share the same rate limit!

With trust proxy:
  Client A (5.6.7.8) → Proxy → Backend sees 5.6.7.8
  Client B (9.10.11.12) → Proxy → Backend sees 9.10.11.12
  ✅ Each client has independent rate limit
```

### CSRF Protection

**Middleware**: `csurf` (double-submit cookie pattern)

**Cookie Behavior Behind Proxy**:
- Cookies require `Secure` flag in production (HTTPS)
- Express automatically uses `Secure` when `trust proxy` is enabled and `X-Forwarded-Proto: https`

**Configuration** (`backend/src/middleware/csrf.ts`):
```typescript
export default csurf({
  cookie: {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production', // Auto-upgrades with trust proxy
    sameSite: 'lax',
  },
});
```

**Why Trust Proxy Matters**:
```
Without trust proxy + TLS termination:
  Backend thinks protocol is HTTP
  → secure: true fails to set cookie
  → CSRF protection broken

With trust proxy + TLS termination:
  Backend reads X-Forwarded-Proto: https
  → secure: true sets cookie correctly
  → CSRF protection works
```

---

## Verification Checklist

### Production Readiness

- [ ] **Environment**: `TRUST_PROXY=true` in production `.env`
- [ ] **Proxy Headers**: Reverse proxy sets `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Real-IP`
- [ ] **TLS Certificate**: Valid cert installed on load balancer
- [ ] **Rate Limiting**: Test with multiple IPs (not all blocked as proxy IP)
- [ ] **CSRF**: Cookies set with `Secure` flag in browser dev tools
- [ ] **CORS**: Console origin allowed in `CORS_ALLOWLIST`
- [ ] **Logs**: Check `req.ip` shows real client IP, not proxy IP

### Testing

```bash
# Test trust proxy is working
curl -H "X-Forwarded-For: 1.2.3.4" https://api.apexmediation.ee/health
# Check logs show 1.2.3.4, not load balancer IP

# Test rate limiting
for i in {1..150}; do
  curl https://api.apexmediation.ee/api/v1/some-endpoint
done
# Should see 429 after 100 requests

# Test CSRF cookie
curl -I https://api.apexmediation.ee/api/v1/csrf-token
# Check Set-Cookie header has Secure flag
```

---

## Common Issues

### Issue 1: Rate limit blocks everyone

**Symptom**: All users get 429 after 100 total requests

**Cause**: `trust proxy` not enabled; backend sees all traffic from proxy IP

**Fix**:
```bash
# In production .env
TRUST_PROXY=true
```

### Issue 2: CSRF cookies not set

**Symptom**: POST requests fail with "invalid csrf token"

**Cause**: Cookie `Secure` flag fails without trust proxy + HTTPS

**Fix**:
```bash
# In production .env
TRUST_PROXY=true
NODE_ENV=production
```

### Issue 3: Wrong client IP in logs

**Symptom**: All logs show same IP (load balancer IP)

**Cause**: `trust proxy` not enabled

**Fix**: Enable trust proxy, verify `X-Forwarded-For` header is set by proxy

---

## Reference

- [Express behind proxies](https://expressjs.com/en/guide/behind-proxies.html)
- [express-rate-limit trust proxy](https://github.com/express-rate-limit/express-rate-limit#:~:text=trust%20proxy)
- [Helmet CSP configuration](https://helmetjs.github.io/)
- [CSRF double-submit cookie](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#double-submit-cookie)
