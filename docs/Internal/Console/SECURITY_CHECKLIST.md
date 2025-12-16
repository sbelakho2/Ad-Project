# Console Security Checklist

## Pre-Deployment Security Review

Use this checklist before deploying console changes to production.

### Authentication

- [ ] **Session Configuration**
  - `COOKIE_SECURE=true` in production
  - `COOKIE_SAMESITE=strict` or `lax`
  - Session expiry set (default: 24 hours)
  - Refresh token rotation enabled

- [ ] **CSRF Protection**
  - CSRF middleware active on mutating endpoints
  - CSRF cookie name matches between backend/console
  - X-CSRF-Token header injected on POST/PUT/PATCH/DELETE

- [ ] **Two-Factor Authentication**
  - `APP_KMS_KEY` set for secret encryption
  - 2FA endpoints rate-limited (5 attempts/minute)
  - Backup codes are bcrypt hashed
  - Login flow handles 2FA challenge

### Authorization (RBAC)

- [ ] **Route Protection**
  - All `/settings/*` routes protected by `settings/layout.tsx`
  - All `/billing/*` routes protected by `billing/layout.tsx`
  - Admin panel redirects to operator control plane
  - 403 page exists and is user-friendly

- [ ] **Role Validation**
  - Only valid roles accepted: `admin`, `publisher`, `readonly`
  - Role strings are case-sensitive
  - Null/undefined roles treated as unauthorized
  - No role escalation via client-side manipulation

- [ ] **Permission Overrides**
  - Billing permissions work for readonly users with `billing:view`
  - Permissions stored in session and validated server-side

### API Security

- [ ] **CORS Configuration**
  - `CORS_ALLOWLIST` contains only trusted origins
  - No wildcard (`*`) in production
  - `withCredentials: true` required

- [ ] **Input Validation**
  - All API inputs validated/sanitized
  - File uploads restricted by type/size
  - Query parameters validated

- [ ] **Rate Limiting**
  - Login attempts rate-limited
  - 2FA attempts rate-limited
  - API requests rate-limited per user

### Operator Separation

- [ ] **Admin Subdomain**
  - `admin.*` hostname returns 404 in console
  - Operator routes redirect to control plane
  - No operator functionality accessible from console

- [ ] **Session Isolation**
  - Customer session ≠ operator session
  - Operator auth uses separate system
  - No cross-session data leakage

---

## Periodic Security Audits

### Monthly

- [ ] Review failed login attempts in logs
- [ ] Check for unusual API patterns
- [ ] Verify no new roles/permissions added without review
- [ ] Audit 2FA adoption rates

### Quarterly

- [ ] Run full RBAC test suite: `npm test -- --testPathPattern="rbac"`
- [ ] Review session timeout settings
- [ ] Check CORS allowlist for stale entries
- [ ] Verify backup codes are rotated by active users

### Annually

- [ ] Full security penetration test
- [ ] RBAC policy review with stakeholders
- [ ] Update dependencies for security patches
- [ ] Review and rotate `APP_KMS_KEY` if needed

---

## Incident Response

### Suspected Account Compromise

1. Force logout all sessions for user
2. Disable 2FA (requires admin)
3. Reset password
4. Review audit logs for unauthorized access
5. Re-enable 2FA with new device

### RBAC Bypass Detected

1. Immediately patch the vulnerability
2. Review access logs for exploitation
3. Notify affected users if data exposed
4. Add regression test for the bypass
5. Update this checklist

### 2FA System Compromise

1. Rotate `APP_KMS_KEY`
2. Force all users to re-enroll 2FA
3. Invalidate all backup codes
4. Review encryption implementation

---

## Testing Commands

```bash
# Run all console tests
cd console && npm test

# Run RBAC-specific tests
cd console && npm test -- --testPathPattern="rbac"

# Run 2FA tests
cd backend && npm test -- --testPathPattern="twofa"

# Run billing access tests
cd console && npm test -- --testPathPattern="billing"

# Build console (catches type errors)
cd console && npm run build
```

---

## Related Documentation

- [RBAC_SYSTEM.md](./RBAC_SYSTEM.md) — Full RBAC implementation details
- [SESSION_AUTH.md](./SESSION_AUTH.md) — Session and CSRF configuration
- [Security/TWO_FACTOR_AUTH.md](../Security/TWO_FACTOR_AUTH.md) — 2FA implementation
- [Security/SECRETS_MANAGEMENT_SETUP.md](../Security/SECRETS_MANAGEMENT_SETUP.md) — Key management
