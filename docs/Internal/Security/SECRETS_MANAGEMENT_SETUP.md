# Secrets Management with Infisical

This document explains the secrets management strategy for the ApexMediation Platform.

## Overview

We use **Infisical** (open-source alternative to AWS Secrets Manager) for centralized secrets management across all environments. All sensitive credentials are stored in Infisical and injected at runtime.

## Why Infisical?

- ✅ **Open Source**: Self-hostable, no vendor lock-in
- ✅ **Free Tier**: Unlimited secrets, 5 team members
- ✅ **Git-like Versioning**: Track secret changes over time
- ✅ **CLI Integration**: Seamless local development
- ✅ **CI/CD Ready**: Native GitHub Actions support
- ✅ **Role-Based Access**: Granular permissions per environment
- ✅ **Audit Logging**: Track who accessed what and when

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    INFISICAL CLOUD                          │
│                 (Secrets Storage & API)                     │
└─────────────────┬───────────────────────────────────────────┘
                  │
        ┌─────────┼─────────┐
        │         │         │
        ▼         ▼         ▼
   ┌─────────┐ ┌──────┐ ┌─────────┐
   │  Local  │ │ CI/CD│ │Production│
   │   Dev   │ │GitHub│ │ Vercel  │
   └─────────┘ └──────┘ └─────────┘
```

## Setup Instructions

### 1. Create Infisical Account

1. Go to https://app.infisical.com/signup
2. Sign up with company email
3. Create organization: "Bel Consulting OÜ"
4. Create project: "ApexMediation Platform"

### 2. Create Environments

Create three environments in Infisical:
- **Development** (local development)
- **Staging** (preview deployments)
- **Production** (live production)

### 3. Initialize Infisical in Backend

```bash
cd backend
infisical login
infisical init
```

Select the project and environment when prompted.

### 4. Migrate Secrets

Run the migration script to copy secrets from `.env` to Infisical:

```bash
# This script will be created in the next step
node scripts/migrate-secrets-to-infisical.js
```

### 5. Update Local Development

Replace `.env` loading with Infisical:

```bash
# Run backend with secrets from Infisical
cd backend
infisical run --env=development -- npm run dev
```

### 6. Update CI/CD

Add Infisical token to GitHub Secrets and update workflows:

```yaml
# .github/workflows/deploy.yml
- name: Run tests with secrets
  run: infisical run --env=staging -- npm test
  env:
    INFISICAL_TOKEN: ${{ secrets.INFISICAL_TOKEN }}
```

## Secret Categories

### Backend Secrets (32 total)

**Server Configuration (3)**
- `PORT` - Server port (4000)
- `NODE_ENV` - Environment (development/staging/production)
- `CORS_ORIGIN` - Allowed CORS origins

**Database (3)**
- `DATABASE_URL` - PostgreSQL connection string
- `CLICKHOUSE_URL` - ClickHouse connection string
- `REDIS_URL` - Redis connection string

**Authentication (2)**
- `JWT_SECRET` - JWT signing key (auto-generated 256-bit)
- `JWT_EXPIRES_IN` - Token expiration (15m for access, 7d for refresh)

**Rate Limiting (2)**
- `RATE_LIMIT_WINDOW_MS` - Rate limit window (900000 = 15min)
- `RATE_LIMIT_MAX_REQUESTS` - Max requests per window (100)

**Logging (1)**
- `LOG_LEVEL` - Log verbosity (debug/info/warn/error)

**AI/ML (5)**
- `OPENAI_API_KEY` - OpenAI API key for AI features
- `ENABLE_AI_AUTOMATION` - Toggle AI automation (true/false)
- `ENABLE_SALES_AI_OPTIMIZATION` - Toggle sales AI (true/false)
- `ENABLE_GROWTH_AI_ANALYTICS` - Toggle growth AI (true/false)
- `ENABLE_SELF_EVOLVING_AI` - Toggle self-evolving AI (true/false)

**Payment Providers (4)**
- `STRIPE_SECRET_KEY` - Adyen secret key
- `STRIPE_WEBHOOK_SECRET` - Adyen webhook signing secret
- `PAYPAL_CLIENT_ID` - PayPal client ID
- `PAYPAL_SECRET` - PayPal secret

**Mailchimp (3)**
- `MAILCHIMP_API_KEY` - Mailchimp API key
- `MAILCHIMP_SERVER_PREFIX` - Mailchimp server prefix (e.g., us12)
- `MAILCHIMP_AUDIENCE_ID` - Mailchimp audience/list ID

**MaxMind GeoIP (2)**
- `MAXMIND_ACCOUNT_ID` - MaxMind account ID
- `MAXMIND_LICENSE_KEY` - MaxMind license key

**Monitoring (4)**
- `SENTRY_DSN` - Sentry error tracking DSN
- `PAGERDUTY_INTEGRATION_KEY` - PagerDuty integration key
- `PAGERDUTY_SERVICE_ID` - PagerDuty service ID
- `CHECKLY_API_KEY` - Checkly uptime monitoring API key

**Other (3)**
- `REFRESH_TOKEN_SECRET` - Separate secret for refresh tokens
- `ENCRYPTION_KEY` - AES-256 key for data encryption
- `WEBHOOK_SIGNING_SECRET` - Secret for signing outbound webhooks

### Website Secrets (12 total)

**API Configuration (2)**
- `NEXT_PUBLIC_API_URL` - Backend API URL
- `NEXT_PUBLIC_SITE_URL` - Website URL (for canonical URLs)

**Analytics (2)**
- `NEXT_PUBLIC_GA_MEASUREMENT_ID` - Google Analytics 4 measurement ID
- `NEXT_PUBLIC_SENTRY_DSN` - Sentry DSN (public, for client-side errors)

**Feature Flags (3)**
- `NEXT_PUBLIC_ENABLE_QUIZ` - Enable quiz feature (true/false)
- `NEXT_PUBLIC_ENABLE_CHAT_WIDGET` - Enable chat widget (true/false)
- `NEXT_PUBLIC_ENABLE_AB_TESTING` - Enable A/B testing (true/false)

**Server-Side Only (5)**
- `SENTRY_AUTH_TOKEN` - Sentry auth token (for uploading source maps)
- `VERCEL_TOKEN` - Vercel API token (for programmatic deployments)
- `INFISICAL_CLIENT_ID` - Infisical Machine Identity client ID
- `INFISICAL_CLIENT_SECRET` - Infisical Machine Identity client secret
- `MAILCHIMP_API_KEY` - Mailchimp API key (for newsletter API route)

## Security Best Practices

### ✅ DO

1. **Use Infisical CLI** for local development
2. **Rotate secrets** every 90 days (set calendar reminders)
3. **Use separate secrets** for each environment
4. **Grant minimum permissions** - developers get staging, not production
5. **Audit access logs** monthly
6. **Use Machine Identities** for CI/CD (not personal tokens)
7. **Enable 2FA** on Infisical account

### ❌ DON'T

1. **Never commit** `.env` files to git (add to `.gitignore`)
2. **Never share secrets** via Slack/email/messages
3. **Never use production secrets** in development
4. **Never hardcode secrets** in source code
5. **Never log secrets** (even in debug mode)
6. **Never store secrets** in CI/CD logs

## Local Development Workflow

### Option 1: Infisical CLI (Recommended)

```bash
# Start backend with secrets injection
cd backend
infisical run --env=development -- npm run dev

# Start website with secrets injection
cd website
infisical run --env=development -- npm run dev

# Run database migrations with secrets
cd backend
infisical run --env=development -- npm run migrate
```

### Option 2: Export to .env (Not Recommended)

```bash
# Export secrets to local .env file (for tools that don't support Infisical)
cd backend
infisical export --env=development > .env

# Remember to delete .env after use!
```

## CI/CD Workflow

### GitHub Actions Integration

1. **Create Machine Identity** in Infisical:
   - Go to Project Settings → Machine Identities
   - Create new identity: "GitHub Actions"
   - Grant access to staging and production environments

2. **Add to GitHub Secrets**:
   - `INFISICAL_CLIENT_ID` - Machine Identity client ID
   - `INFISICAL_CLIENT_SECRET` - Machine Identity client secret

3. **Update workflows**:

```yaml
# .github/workflows/backend-deploy.yml
name: Deploy Backend

on:
  push:
    branches: [main, develop]

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
      
      - name: Install Infisical CLI
        run: |
          curl -1sLf 'https://dl.cloudsmith.io/public/infisical/infisical-cli/setup.deb.sh' | sudo -E bash
          sudo apt-get update && sudo apt-get install -y infisical
      
      - name: Authenticate with Infisical
        run: infisical login --method=universal-auth --client-id=${{ secrets.INFISICAL_CLIENT_ID }} --client-secret=${{ secrets.INFISICAL_CLIENT_SECRET }}
      
      - name: Install dependencies
        run: npm ci
      
      - name: Run tests with secrets
        run: infisical run --env=staging -- npm test
      
      - name: Deploy to production
        if: github.ref == 'refs/heads/main'
        run: infisical run --env=production -- npm run deploy
```

### Vercel Integration (Website)

1. **Install Infisical Vercel Integration**:
   - Go to Infisical project → Integrations
   - Click "Add Integration" → Select "Vercel"
   - Authorize Vercel
   - Select Vercel project
   - Map environments:
     - Development → Preview
     - Staging → Preview
     - Production → Production

2. **Secrets auto-sync** to Vercel environment variables on every change

## Secret Rotation Schedule

| Secret | Rotation Frequency | Owner |
|--------|-------------------|-------|
| JWT_SECRET | Every 90 days | DevOps Lead |
| STRIPE_SECRET_KEY | Every 180 days | Finance Lead |
| OPENAI_API_KEY | Every 90 days | AI/ML Lead |
| DATABASE_URL | On breach only | Database Admin |
| MAILCHIMP_API_KEY | Every 180 days | Marketing Lead |
| All others | Annually | DevOps Lead |

## Disaster Recovery

### Scenario 1: Infisical Outage

**Fallback**: Keep encrypted backup of production secrets in a local vault (KeePassXC `.kdbx`) or UNIX `pass` (GPG-backed).

**Steps**:
1. Open KeePassXC vault (.kdbx) or initialize/access the UNIX `pass` store (GPG)
2. Decrypt and retrieve required secrets using the master password/key file (or GPG key)
3. Export to a temporary `.env` file on the admin host
4. Deploy with `.env` until Infisical recovers (prefer DO App Secrets for runtime)
5. Securely delete the temporary `.env` after recovery

### Scenario 2: Leaked Secret

**Steps**:
1. Immediately rotate the leaked secret in Infisical
2. Update all services using that secret
3. Review audit logs to identify leak source
4. Revoke compromised access
5. Incident post-mortem within 24 hours

### Scenario 3: Lost Infisical Access

**Prevention**: Maintain 2 admin accounts (different people, different 2FA devices)

**Recovery**:
1. Contact Infisical support with ownership proof
2. Use backup admin account
3. Fallback to KeePassXC/`pass` encrypted backup (see Scenario 1)

## Migration Checklist

- [x] Install Infisical CLI (`brew install infisical/get-cli/infisical`)
- [ ] Create Infisical account and project
- [ ] Create 3 environments (development, staging, production)
- [ ] Run migration script to copy secrets
- [ ] Update backend startup script to use `infisical run`
- [ ] Update website startup script to use `infisical run`
- [ ] Create Machine Identity for GitHub Actions
- [ ] Add Infisical tokens to GitHub Secrets
- [ ] Update all GitHub Actions workflows
- [ ] Set up Vercel integration for website
- [ ] Test secret access in all environments
- [ ] Remove `.env` files from git history (use BFG Repo-Cleaner)
- [ ] Add `.env*` to `.gitignore` (if not already)
- [ ] Document rotation schedule
- [ ] Set up encrypted backup in KeePassXC or `pass`
- [ ] Schedule quarterly secret rotation reminders
- [ ] Train team on Infisical usage

## Cost

**Free Tier** (current usage):
- Unlimited secrets ✅
- 5 team members ✅
- 3 environments ✅
- Basic RBAC ✅
- 30-day audit logs ✅

**Pro Tier** ($18/user/month, optional upgrades):
- Advanced RBAC
- 1-year audit logs
- Custom roles
- Priority support
- SAML SSO

**Recommendation**: Start with Free tier, upgrade to Pro when team > 5 people.

## Support

**Infisical Documentation**: https://infisical.com/docs  
**Community Slack**: https://infisical.com/slack  
**Status Page**: https://status.infisical.com

---

**Document Owner**: DevOps Lead  
**Last Updated**: January 2025  
**Next Review**: April 2025
