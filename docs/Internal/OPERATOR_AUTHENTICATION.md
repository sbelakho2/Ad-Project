# Operator Authentication

The Admin Control Panel (operator cockpit) uses a **separate authentication system** from customer authentication. This ensures that no customer - regardless of their RBAC role - can access company-privileged data or system controls.

## ⚠️ CRITICAL SECURITY NOTICE

The customer `role = 'admin'` is a **customer role** within their organization. It does NOT grant access to the operator panel. The admin panel requires **operator authentication** which is completely separate.

## Authentication Methods

### 1. Operator Email Whitelist (Recommended for Production)

Set the `NEXT_PUBLIC_OPERATOR_EMAILS` environment variable with a comma-separated list of authorized operator email addresses:

```bash
NEXT_PUBLIC_OPERATOR_EMAILS=operator@apexmediation.com,admin@apexmediation.com,sabelakhoua@apexmediation.com
```

Users with whitelisted emails must be authenticated via the regular session system (`/api/auth/session`), but only their emails are checked against the whitelist.

### 2. Operator Secret Key (For Development/Staging)

Set the `NEXT_PUBLIC_OPERATOR_SECRET` environment variable:

```bash
NEXT_PUBLIC_OPERATOR_SECRET=your-secure-secret-key-here
```

Operators can then log in at `/admin/login` using this secret.

**⚠️ Important:** The operator secret is exposed client-side via `NEXT_PUBLIC_`. For production, use the email whitelist method or implement server-side validation.

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `NEXT_PUBLIC_OPERATOR_EMAILS` | Comma-separated list of operator email addresses | `admin@company.com,ops@company.com` |
| `NEXT_PUBLIC_OPERATOR_SECRET` | Secret key for operator login (dev/staging only) | `super-secret-key-123` |

## Access Flow

1. User navigates to `/admin/*`
2. `useAdminGate` hook checks:
   - Is there a valid operator secret in session storage?
   - Is the authenticated user's email in the operator whitelist?
3. If neither check passes, user is redirected to `/admin/login`
4. Operator authenticates via secret key or whitelisted email
5. Access is granted to the operator control panel

## Admin Panel Features (Operator Only)

- **Health**: API/RTB metrics, RED dashboard, latency monitoring
- **Infrastructure**: Database autopilot, partition management, connection pools
- **Billing Ops**: Internal billing operations and reconciliation
- **Audit Log**: System-wide audit trail
- **Sales Automation**: Lead scoring and automation rules
- **Value Multipliers**: Revenue optimization controls

## Security Considerations

1. **Fail-Secure**: If no operator emails are configured, ALL access is denied
2. **Session Storage**: Operator credentials are stored in `sessionStorage` (cleared on browser close)
3. **Audit Logging**: All operator actions should be logged (implement in each feature)
4. **IP Whitelist**: Consider adding IP-based restrictions for production

## Deployment Checklist

For the operator authentication to work in production (Hetzner servers), add these environment variables:

### On App Server (46.62.252.96)

Add to the Next.js environment (e.g., `.env.production` or container environment):

```bash
# Required: Comma-separated list of operator email addresses
NEXT_PUBLIC_OPERATOR_EMAILS=sabelakhoua@apexmediation.ee,operator@apexmediation.ee

# Optional: Secret key for operator login (for development/staging only)
# NEXT_PUBLIC_OPERATOR_SECRET=your-secure-secret-here
```

After adding, restart the console service:

```bash
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
# Add environment variables and restart service
```

### Backend API Authorization

The backend admin routes (`/api/v1/admin/*`) still use customer role authorization (`authorize(['admin'])`). For full separation:

1. Operators authenticate normally as customers with `admin` role, OR
2. Implement backend operator middleware (future enhancement)

Currently, the frontend operator gate prevents customer admins from accessing the UI, but the API itself still accepts customer admin tokens.

## Future Enhancements

- OAuth/SSO integration with corporate identity provider
- Hardware security key (WebAuthn) support
- IP-based access restrictions
- Time-based access policies
- Multi-factor authentication for operators
