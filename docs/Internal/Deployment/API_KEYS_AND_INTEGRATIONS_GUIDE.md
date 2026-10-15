# ApexMediation API Keys & Integrations Guide

Complete step-by-step instructions for obtaining and configuring all required API keys, payment provider credentials, and third-party service integrations for production deployment.

---

## Table of Contents

1. [Payment Providers](#payment-providers)
2. [Email & Communication](#email--communication)
3. [Analytics & Monitoring](#analytics--monitoring)
4. [Ad Networks](#ad-networks)
5. [Infrastructure & Security](#infrastructure--security)
6. [Environment Variable Setup](#environment-variable-setup)
7. [Testing & Verification](#testing--verification)

---

## Payment Providers

### 1. Adyen (Primary Payout Solution)

**Purpose:** Process payouts to developers via Adyen Connect.

#### Setup Instructions:

1. **Create Adyen Account**
   - Go to https://dashboard.adyen.com/register
   - Sign up with email and password
   - Verify email address

2. **Activate Adyen Connect**
   - Navigate to `Settings` → `Connect settings`
   - Enable "Express accounts" for publisher onboarding
   - Configure redirect URLs:
     - Development: `http://localhost:3000/adyen-callback`
     - Production: `https://console.apexmediation.ee/adyen-callback`

3. **Get API Keys**
   - Go to `Developers` → `API keys`
   - You'll find:
     - **Publishable Key** (starts with `pk_test_` or `pk_live_`)
     - **Secret Key** (starts with `sk_test_` or `sk_live_`)
   - Click "Reveal test key" or switch to Live keys (requires activation)

4. **For Testing (Sandbox)**
   - Use test keys in development
   - Test cards: `4242 4242 4242 4242` (expiry: any future date, CVC: any 3 digits)
   - Simulate payouts without real money

5. **For Production (Live)**
   - Complete Adyen verification (business info, tax ID)
   - Switch to Live keys
   - Enable webhook signing

6. **Set Webhook Endpoints**
   - Go to `Developers` → `Webhooks`
   - Add endpoint: `https://api.apexmediation.ee/webhooks/adyen`
   - Subscribe to events:
     - `account.updated`
     - `charge.refunded`
     - `transfer.created`
     - `transfer.failed`

#### Environment Variables:
```bash
APEX_ADYEN_API_KEY=sk_live_XXXXXXXXXXXXXXXXXXXXXXXX
APEX_ADYEN_MERCHANT_ACCOUNT=pk_live_XXXXXXXXXXXXXXXXXXXXXXXX
APEX_ADYEN_HMAC_KEY=whsec_XXXXXXXXXXXXXXXXXXXXXXXX
```

#### Documentation:
- https://adyen.com/docs/connect
- https://adyen.com/docs/connect/onboarding/hosted-onboarding

---

### 2. PayPal (Alternative Payout Solution)

**Purpose:** Process mass payouts to developers via PayPal.

#### Setup Instructions:

1. **Create PayPal Business Account**
   - Go to https://www.paypal.com/en/business/accept-payments
   - Click "Get Started"
   - Sign up as a business

2. **Access Developer Dashboard**
   - Log in to https://developer.paypal.com
   - Go to `Apps & Credentials`
   - Make sure you're in "Sandbox" mode (top-right toggle)

3. **Create Application**
   - Click "Create App" under "REST API apps"
   - Name: `ApexMediation Payouts`
   - Type: `Merchant`
   - Click "Create App"

4. **Get Sandbox Credentials**
   - In the app details, find:
     - **Client ID** (under "Sandbox")
     - **Secret** (click "Show")
   - Example format:
     - Client ID: `ARPvB...`
     - Secret: `EMcfH...`

5. **For Production**
   - Switch toggle from "Sandbox" to "Live" (top-right)
   - Get production Client ID and Secret
   - Complete business verification in PayPal Business account

6. **Configure Payouts Permissions**
   - Go to `Account Settings` → `Permissions`
   - Grant permission for "Mass Pay" or "Payouts API"

#### Environment Variables:
```bash
PAYPAL_CLIENT_ID=ARPvB_xxxxxxxxxxxxxxxxxxxxxxxx
PAYPAL_CLIENT_SECRET=EMcfH_xxxxxxxxxxxxxxxxxxxxxxxx
PAYPAL_MODE=live  # or 'sandbox' for testing
```

#### Documentation:
- https://developer.paypal.com/docs/api/overview/
- https://developer.paypal.com/docs/api/reference/get-an-access-token-curl/

---

## Email & Communication

### SendGrid (Email Service)

**Purpose:** Send automated emails (confirmations, payouts, alerts).

#### Setup Instructions:

1. **Create SendGrid Account**
   - Go to https://sendgrid.com/pricing
   - Click "Start Free" (free tier: 40,000 emails/month)
   - Sign up with email

2. **Verify Sender Email**
   - Go to `Sender authentication` → `Single Sender Verification`
   - Click "Create New Sender"
   - Fill in from address: `noreply@apexmediation.ee`
   - Click verification link in email from SendGrid

3. **Generate API Key**
   - Go to `Settings` → `API Keys`
   - Click "Create API Key"
   - Name: `ApexMediation Backend`
   - Permissions: Select "Restricted Access"
   - Enable: `Mail Send` permission only
   - Copy the key (won't be shown again)

4. **Set Up Email Templates (Optional)**
   - Go to `Dynamic Templates`
   - Create templates for:
     - Welcome email
     - Payout confirmation
     - Account alerts
     - Password reset

#### Environment Variables:
```bash
SENDGRID_API_KEY=SG.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
SENDGRID_FROM_EMAIL=noreply@apexmediation.ee
SENDGRID_FROM_NAME=ApexMediation
```

#### Documentation:
- https://sendgrid.com/docs/for-developers/sending-email/quickstart-nodejs/
- https://sendgrid.com/docs/ui/account-and-settings/api-keys/

---

## Analytics & Monitoring

### 1. Google Analytics 4

**Purpose:** Track website traffic, user behavior, conversions.

#### Setup Instructions:

1. **Create Google Analytics Account**
   - Go to https://analytics.google.com
   - Sign in with Google account
   - Click "Start measuring"

2. **Set Up Property**
   - Account name: `ApexMediation`
   - Property name: `Website`
   - Time zone: `Europe/Tallinn`
   - Currency: `EUR`

3. **Create Web Data Stream**
   - Platform: Web
   - Website URL: `https://apexmediation.ee`
   - Stream name: `Main Website`
   - Click "Create stream"

4. **Get Measurement ID**
   - After stream is created, find:
     - **Measurement ID**: `G-XXXXXXXXXX`
   - This appears in the "Web stream details" section

5. **Install Google Tag Manager (Optional but Recommended)**
   - Easier way to manage analytics, ads pixels, etc.
   - Go to https://tagmanager.google.com
   - Create account for ApexMediation
   - Get **Container ID**: `GTM-XXXXXXX`

#### Environment Variables:
```bash
NEXT_PUBLIC_GA_ID=G-XXXXXXXXXX
NEXT_PUBLIC_GTM_ID=GTM-XXXXXXX  # Optional
```

#### Documentation:
- https://support.google.com/analytics/answer/9539674
- https://developers.google.com/analytics/devguides/collection/ga4

---

### 2. Sentry (Error Tracking - Recommended)

**Purpose:** Monitor errors and performance issues in production.

#### Setup Instructions:

1. **Create Sentry Account**
   - Go to https://sentry.io/signup
   - Sign up with email or GitHub
   - Create organization

2. **Create Projects**
   - Create project for Backend (Node.js)
   - Create project for Website (React)
   - Create project for Go services

3. **Get DSN (Data Source Name)**
   - In each project, go to `Settings` → `Client Keys (DSN)`
   - Copy the DSN URL
   - Format: `https://xxxxx@xxxxx.ingest.sentry.io/xxxxxx`

4. **Configure Alert Rules** (Optional)
   - Go to `Alerts` → `Create Alert Rule`
   - Notify Slack when errors occur
   - Escalate for critical issues

#### Environment Variables:
```bash
# Backend
SENTRY_DSN_BACKEND=https://xxxxx@xxxxx.ingest.sentry.io/xxxxxx

# Website
NEXT_PUBLIC_SENTRY_DSN=https://xxxxx@xxxxx.ingest.sentry.io/xxxxxx

# Alert Slack webhook (optional)
SENTRY_SLACK_WEBHOOK=https://hooks.slack.com/services/...
```

#### Documentation:
- https://docs.sentry.io/platforms/node/
- https://docs.sentry.io/platforms/javascript/guides/nextjs/

---

### 3. Hotjar (Session Recording - Optional)

**Purpose:** Record user sessions to understand user behavior.

#### Setup Instructions:

1. **Create Hotjar Account**
   - Go to https://www.hotjar.com
   - Click "Start free trial" (up to 1,000 sessions/month)
   - Sign up with email

2. **Add Site**
   - Go to `Sites & organizations`
   - Click "Add site"
   - Site URL: `https://apexmediation.ee`
   - Industry: `SaaS`

3. **Get Hotjar ID**
   - After site is created, find Tracking Code
   - Extract **Hotjar ID**: `hjid=XXXXXXX`
   - Format: A number like `hjid=2891827`

4. **Set Recording Options**
   - Go to `Site settings` → `Record and playback`
   - Enable: "Record user sessions"
   - Set daily session limit
   - Configure what data to capture (optional)

#### Environment Variables:
```bash
NEXT_PUBLIC_HOTJAR_ID=2891827
```

#### Documentation:
- https://help.hotjar.com/hc/en-us/articles/115011640207-Hotjar-Tracking-Code

---

## Ad Networks

### Adapter Configuration (Per Publisher)

These credentials are stored **per publisher** in the `adapter_configs` PostgreSQL table, not as global env vars.

#### 1. ironSource

**Purpose:** Fill ad impressions via ironSource demand sources.

**Where to Get Credentials:**
- Go to https://platform.ironsrc.com
- Sign up / Log in
- Navigate to `Setup & Integration` → `SDK Networks`
- Select iOS/Android
- Find **App Key**: `xxxxxxxx`

**Configuration:**
```json
{
  "appKey": "xxxxxxxx",
  "userId": "user_id_from_ironsource",
  "segment": "default"
}
```

#### 2. AppLovin

**Purpose:** Fill ad impressions via AppLovin demand sources.

**Where to Get Credentials:**
- Go to https://www.applovin.com/dashboard
- Sign up / Log in
- Go to `Account` → `Keys`
- Find **SDK Key**: `xxxxxxxxxxxxx`

**Configuration:**
```json
{
  "sdkKey": "xxxxxxxxxxxxx",
  "mediationKey": "xxxxxxxxxxxxx"
}
```

#### 3. Unity Ads

**Purpose:** Fill ad impressions via Unity Ads network.

**Where to Get Credentials:**
- Go to https://unityads.unity3d.com
- Sign up / Log in
- Go to `Projects` → Select your game project
- Find **Game ID**: `xxxxxxx`

**Configuration:**
```json
{
  "gameId": "xxxxxxx",
  "placementId": "Rewarded_iOS"
}
```

Note: Use the appropriate placementId for platform, e.g., "Rewarded_Android" on Android.

#### 4. Google AdMob

**Purpose:** Fill ad impressions via Google AdMob network.

**Where to Get Credentials:**
- Go to https://admob.google.com
- Sign in with Google
- Go to `Apps` → Create new app
- Add app (select platform: iOS/Android)
- Create ad units
- Find **App ID**: `ca-app-pub-xxxxxxxxxxxxxxxx~yyyyyyyyyy`

**Configuration:**
```json
{
  "appId": "ca-app-pub-xxxxxxxxxxxxxxxx~yyyyyyyyyy",
  "interstitialUnitId": "ca-app-pub-3940256099942544~1033173712",
  "rewardedUnitId": "ca-app-pub-3940256099942544~5224354917"
}
```

#### 5. Meta Audience Network

**Purpose:** Fill ad impressions via Meta (Facebook) audience network.

**Where to Get Credentials:**
- Go to https://developers.facebook.com
- Create app (type: Business)
- Add Audience Network product
- Create placements
- Find **Placement ID**: `YOUR_PLACEMENT_ID`

**Configuration:**
```json
{
  "placementId": "YOUR_PLACEMENT_ID",
  "appId": "YOUR_APP_ID"
}
```

---

## Infrastructure & Security

### 1. JWT Secret

**Purpose:** Sign and verify JWT authentication tokens.

#### How to Generate:
```bash
# On Mac/Linux:
openssl rand -base64 32

# Or using Node:
node -e "console.log(require('crypto').randomBytes(32).toString('hex'))"

# Result example: 7f8a9c2b1e5d4f6a8c3e1b9d7f2a4e6c8b1d3f5a7c9e2b4d6f8a0c2e4g6h8
```

#### Environment Variables:
```bash
JWT_SECRET=7f8a9c2b1e5d4f6a8c3e1b9d7f2a4e6c8b1d3f5a7c9e2b4d6f8a0c2e4g6h8
JWT_EXPIRES_IN=7d
```

---

### 2. AWS Credentials (For Cloud Deployment)

**Purpose:** Deploy to AWS ECS/EKS, store files in S3, etc.

#### Setup Instructions:

1. **Create AWS Account**
   - Go to https://aws.amazon.com
   - Click "Create an AWS Account"
   - Complete registration

2. **Create IAM User**
   - Go to `IAM` → `Users` → `Create User`
   - User name: `apexmediation-ci`
   - Enable "Access key - Programmatic access"

3. **Set Permissions**
   - Attach policy: `AmazonECS_FullAccess`
   - Attach policy: `AmazonEC2FullAccess`
   - Attach policy: `AmazonEKS_CNI_Policy`

4. **Get Access Keys**
   - After user creation, find:
     - **Access Key ID**: `AKIAIOSFODNN7EXAMPLE`
     - **Secret Access Key**: (download CSV, won't be shown again)

#### Environment Variables:
```bash
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
AWS_REGION=eu-west-1
AWS_ACCOUNT_ID=123456789012
```

#### Documentation:
- https://docs.aws.amazon.com/IAM/latest/UserGuide/id_users_create.html

---

### 3. Slack Webhook (Deployment Notifications)

**Purpose:** Send notifications to Slack when deployments happen.

#### Setup Instructions:

1. **Create Slack Workspace** (if you don't have one)
   - Go to https://slack.com
   - Click "Create a new workspace"
   - Follow setup wizard

2. **Create Channel**
   - In Slack, create new channel: `#deployments`
   - Make it private

3. **Create Incoming Webhook**
   - Go to https://api.slack.com/apps
   - Click "Create New App"
   - Select "From scratch"
   - App name: `ApexMediation CI/CD`
   - Workspace: Your workspace

4. **Enable Incoming Webhooks**
   - Go to `Incoming Webhooks`
   - Toggle "Activate Incoming Webhooks" to ON
   - Click "Add New Webhook to Workspace"
   - Select channel: `#deployments`
   - Authorize

5. **Copy Webhook URL**
   - Format: `https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXX`

#### Environment Variables:
```bash
SLACK_WEBHOOK_URL=https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXX
```

#### Documentation:
- https://api.slack.com/messaging/webhooks

---

### 4. GitHub (For CI/CD)

**Purpose:** Store code, run automated tests, deploy on push.

#### Setup Instructions:

1. **Create GitHub Account**
   - Go to https://github.com
   - Sign up with email

2. **Create Repository**
   - Click "New repository"
   - Name: `apexmediation`
   - Private (recommended)
   - Initialize with README

3. **Add Secrets for Workflows**
   - Go to `Settings` → `Secrets and variables` → `Actions`
   - Click "New repository secret"
   - Add secrets for:
     - `APEX_ADYEN_API_KEY`
     - `PAYPAL_CLIENT_ID`
     - `AWS_ACCESS_KEY_ID`
     - `SLACK_WEBHOOK_URL`
     - etc.

4. **Create Deployment Key (Optional)**
   - Go to `Settings` → `Deploy keys`
   - Generate SSH key: `ssh-keygen -t ed25519`
   - Add public key to GitHub

#### Environment Variables:
```bash
GITHUB_TOKEN=ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

#### Documentation:
- https://docs.github.com/en/actions/security-guides/encrypted-secrets

---

## Environment Variable Setup

### 1. Backend Environment File

Create `.env` in `/backend`:

```bash
# Environment
NODE_ENV=production

# Server
PORT=4000
API_VERSION=v1
CORS_ORIGIN=https://console.apexmediation.ee

# Database
DATABASE_URL=postgresql://user:pass@db.apexmediation.ee:5432/apexmediation
DATABASE_SSL=true
DATABASE_POOL_MAX=20

# ClickHouse
CLICKHOUSE_URL=http://clickhouse.apexmediation.ee:8123
CLICKHOUSE_DATABASE=apexmediation
CLICKHOUSE_USER=default
CLICKHOUSE_PASSWORD=secure_password

# Redis
REDIS_URL=redis://redis.apexmediation.ee:6379

# JWT
JWT_SECRET=7f8a9c2b1e5d4f6a8c3e1b9d7f2a4e6c8b1d3f5a7c9e2b4d6f8a0c2e4g6h8
JWT_EXPIRES_IN=7d

# Rate Limiting
RATE_LIMIT_WINDOW_MS=900000
RATE_LIMIT_MAX_REQUESTS=100

# Logging
LOG_LEVEL=info

# Payment Providers
APEX_ADYEN_API_KEY=sk_live_XXXXXXXXXXXXXXXXXXXXXXXX
APEX_ADYEN_HMAC_KEY=whsec_XXXXXXXXXXXXXXXXXXXXXXXX
PAYPAL_CLIENT_ID=ARPvB_xxxxxxxxxxxxxxxxxxxxxxxx
PAYPAL_CLIENT_SECRET=EMcfH_xxxxxxxxxxxxxxxxxxxxxxxx
PAYPAL_MODE=live

# Email
SENDGRID_API_KEY=SG.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
SENDGRID_FROM_EMAIL=noreply@apexmediation.ee
SENDGRID_FROM_NAME=ApexMediation

# Error Tracking
SENTRY_DSN_BACKEND=https://xxxxx@xxxxx.ingest.sentry.io/xxxxxx

# Slack
SLACK_WEBHOOK_URL=https://hooks.slack.com/services/...

# AWS
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
AWS_REGION=eu-west-1
```

### 2. Website Environment File

Create `.env.local` in `/website`:

```bash
# API Endpoints
NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/v1
NEXT_PUBLIC_BACKEND_URL=https://api.apexmediation.ee

# Site
NEXT_PUBLIC_SITE_URL=https://apexmediation.ee
NEXT_PUBLIC_CONSOLE_URL=https://console.apexmediation.ee

# Analytics
NEXT_PUBLIC_GA_ID=G-XXXXXXXXXX
NEXT_PUBLIC_HOTJAR_ID=2891827
NEXT_PUBLIC_SENTRY_DSN=https://xxxxx@xxxxx.ingest.sentry.io/xxxxxx

# Feature Flags
NEXT_PUBLIC_ENABLE_CHAT=true
NEXT_PUBLIC_ENABLE_BLOG=true

# Server-side only (no NEXT_PUBLIC prefix)
API_SECRET_KEY=your_secret_key
SENDGRID_API_KEY=SG.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
APEX_ADYEN_API_KEY=sk_live_XXXXXXXXXXXXXXXXXXXXXXXX
```

### 3. Go Services Environment Files

Create `.env` for `/backend/router`, `/backend/analytics`, etc.:

```bash
PORT=8085
ENVIRONMENT=production

# Database
POSTGRES_URL=postgresql://user:pass@db.apexmediation.ee:5432/apexmediation

# Redis
REDIS_ADDR=redis.apexmediation.ee:6379
REDIS_PASSWORD=secure_password

# ClickHouse (Analytics)
CLICKHOUSE_ADDR=clickhouse.apexmediation.ee:9000
CLICKHOUSE_DB=apexmediation

# GeoIP
GEOIP_DB_PATH=/data/GeoLite2-Country.mmdb

# Logging
LOG_LEVEL=info
```

---

## Testing & Verification

### 1. Test Adyen Integration

```bash
# Test creating a Adyen Connect account
curl -X POST https://api.apexmediation.ee/v1/publishers/123/adyen-account \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "email": "publisher@example.com",
    "country": "EE"
  }'

# Expected response:
# {
#   "success": true,
#   "adyenAccountId": "acct_1234567890"
# }
```

### 2. Test PayPal Integration

```bash
# Test mass payout
curl -X POST https://api.apexmediation.ee/v1/payouts/process \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "publisherId": "123",
    "amount": 100.00,
    "currency": "EUR",
    "method": "paypal",
    "email": "publisher@example.com"
  }'
```

### 3. Test SendGrid

```bash
# Test sending email
curl -X POST https://api.sendgrid.com/v3/mail/send \
  -H "Authorization: Bearer SG.your_api_key" \
  -H "Content-Type: application/json" \
  -d '{
    "personalizations": [{"to": [{"email": "test@example.com"}]}],
    "from": {"email": "noreply@apexmediation.ee"},
    "subject": "Test Email",
    "content": [{"type": "text/plain", "value": "Hello World"}]
  }'
```

### 4. Test Google Analytics

- Deploy website with `NEXT_PUBLIC_GA_ID` set
- Go to https://analytics.google.com
- Select your property
- Go to `Real-time` → `Overview`
- Visit your website, you should see active user

### 5. Verify All Env Vars

```bash
# Backend
printenv | grep -E "ADYEN|PAYPAL|SENDGRID|JWT|DATABASE|REDIS"

# Website (in .env.local)
grep NEXT_PUBLIC website/.env.local
```

---

## Checklist Before Launch

- [ ] Adyen account created and verified
- [ ] Adyen Connect onboarding working
- [ ] Adyen webhook configured
- [ ] PayPal credentials obtained
- [ ] SendGrid account and API key ready
- [ ] Google Analytics 4 property created
- [ ] Sentry projects set up (optional)
- [ ] Hotjar account created (optional)
- [ ] AWS IAM user created with credentials
- [ ] Slack webhook URL configured
- [ ] All env vars set in production
- [ ] Database migrations run successfully
- [ ] Backend deployment verified
- [ ] Website deployment verified
- [ ] Payment provider tests passing
- [ ] Email delivery working
- [ ] Analytics events tracking

---

## Support & Documentation

- **Adyen Support:** https://support.adyen.com/
- **PayPal Support:** https://www.paypal.com/en/smarthelp
- **SendGrid Support:** https://support.sendgrid.com/
- **Google Analytics:** https://support.google.com/analytics/
- **AWS Support:** https://console.aws.amazon.com/support/

---

**Last Updated:** 2025-11-04
**Maintained by:** ApexMediation Team


---

### Fyber (Digital Turbine FairBid) — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- FYBER_APP_ID — application identifier
- FYBER_API_KEY — API key (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/fyber.go
- Supports test_endpoint override for offline tests
- Standardized retry + jitter + circuit breaker; normalized NoBid taxonomy

Configuration example (dev):
```
FYBER_APP_ID=your_app_id
FYBER_API_KEY=your_api_key
```

Notes:
- Do not paste secrets into code or docs. Use environment variables.
- Sandbox/official endpoints to be configured during FT phase.

### Appodeal — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- APPODEAL_APP_KEY — SDK/app key

Adapter wiring (Go backend): backend/auction/internal/bidders/appodeal.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; circuit breaker enabled

Configuration example (dev):
```
APPODEAL_APP_KEY=your_appodeal_key
```

### Admost — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- ADMOST_APP_ID — application identifier
- ADMOST_API_KEY — API key (bearer)

Adapter wiring (Go backend): backend/auction/internal/bidders/admost.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; circuit breaker enabled

Configuration example (dev):
```
ADMOST_APP_ID=your_app_id
ADMOST_API_KEY=your_api_key
```

Operational notes:
- All new adapters adhere to the normalized NoBid taxonomy: timeout, network_error, status_XXX, no_fill, circuit_open, error.
- Offline conformance tests for these adapters live in backend/auction/internal/bidders/adapter_conformance_test.go
- Production/sandbox credentials and endpoint validation occur in FT phase per DEVELOPMENT_ROADMAP.md


### Chocolate Platform — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- CHOCOLATE_APP_ID — application identifier
- CHOCOLATE_API_KEY — API key (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/chocolate.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; circuit breaker enabled; metrics/tracing/debugger hooks

Configuration example (dev):
```
CHOCOLATE_APP_ID=your_app_id
CHOCOLATE_API_KEY=your_api_key
```

Notes:
- Do not paste secrets into code or docs. Use environment variables.
- Sandbox/official endpoints to be configured during FT phase.

### Tapdaq — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- TAPDAQ_APP_ID — application identifier
- TAPDAQ_API_KEY — API key (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/tapdaq.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; circuit breaker enabled; metrics/tracing/debugger hooks

Configuration example (dev):
```
TAPDAQ_APP_ID=your_app_id
TAPDAQ_API_KEY=your_api_key
```

Operational notes:
- Adheres to normalized NoBid taxonomy: timeout, network_error, status_XXX, no_fill, circuit_open, error.
- Offline conformance tests for these adapters live in backend/auction/internal/bidders/chocolate_tapdaq_conformance_test.go
- Production/sandbox credentials and endpoint validation occur in FT phase per DEVELOPMENT_ROADMAP.md


### Vungle — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- VUNGLE_APP_ID — application identifier
- VUNGLE_API_KEY — API key or bearer token (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/vungle.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; CircuitBreaker enabled; metrics/tracing/debugger hooks

Configuration example (dev):
```
VUNGLE_APP_ID=your_app_id
VUNGLE_API_KEY=your_api_key
```

Notes:
- Do not paste secrets into code or docs. Use environment variables.
- Offline conformance tests live in backend/auction/internal/bidders/vungle_conformance_test.go
- Sandbox/official endpoints to be configured during FT phase.

### Pangle — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- PANGLE_APP_ID — application identifier
- PANGLE_API_KEY — API key (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/pangle.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; CircuitBreaker enabled; metrics/tracing/debugger hooks

Configuration example (dev):
```
PANGLE_APP_ID=your_app_id
PANGLE_API_KEY=your_api_key
```

Operational notes:
- Adheres to normalized NoBid taxonomy: timeout, network_error, status_XXX, no_fill, circuit_open, error.
- Conformance tests to mirror Vungle’s suite; implemented adapter compiles and follows shared patterns.
- Production/sandbox credentials and endpoint validation occur in FT phase per DEVELOPMENT_ROADMAP.md


---

### Mintegral — Development placeholders
Purpose: Enable S2S bidding via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- MINTEGRAL_APP_ID — application identifier
- MINTEGRAL_API_KEY — API key (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/mintegral.go
- Supports test_endpoint override for offline tests
- Standardized retry + jitter + circuit breaker; normalized NoBid taxonomy; metrics/tracing/debugger hooks

Configuration example (dev):
```
MINTEGRAL_APP_ID=your_app_id
MINTEGRAL_API_KEY=your_api_key
```

Notes:
- Do not paste secrets into code or docs. Use environment variables.
- Sandbox/official endpoints to be configured during FT phase.

### Moloco — Development placeholders
Purpose: Enable Moloco Cloud demand via placeholder adapter for offline conformance and dev.

Required fields (dev/local):
- MOLOCO_SEAT_ID — Moloco seat identifier
- MOLOCO_API_KEY — API key (mask in logs; do not commit)

Adapter wiring (Go backend): backend/auction/internal/bidders/moloco.go
- Supports test_endpoint override for offline tests
- Standardized resiliency and taxonomy; circuit breaker enabled; metrics/tracing/debugger hooks

Configuration example (dev):
```
MOLOCO_SEAT_ID=your_seat
MOLOCO_API_KEY=your_api_key
```

---

## Adapter Compatibility Matrix (S2S bidders)

| Adapter                | Bidding Type | Formats                          | Surfaces              | Keys/Params                         | Default Timeout |
|------------------------|--------------|-----------------------------------|-----------------------|-------------------------------------|-----------------|
| AdMob                  | S2S          | banner, interstitial, rewarded    | iOS, Android, Web     | API key, Publisher ID               | 5s              |
| ironSource LevelPlay   | S2S          | interstitial, rewarded, banner    | iOS, Android          | App Key, Placement/Instance IDs     | 5s              |
| AppLovin MAX           | S2S          | interstitial, rewarded, banner    | iOS, Android          | SDK Key, Ad Unit ID                 | 5s              |
| Unity Mediation        | S2S          | interstitial, rewarded            | iOS, Android          | Game ID/App ID, Placement ID        | 5s              |
| Fyber FairBid          | S2S          | interstitial, rewarded, banner    | iOS, Android          | App ID, API Key                     | 5s              |
| Appodeal               | S2S          | interstitial, rewarded, banner    | iOS, Android          | App Key                             | 5s              |
| Admost                 | S2S          | interstitial, rewarded, banner    | iOS, Android          | App ID, API Key                     | 5s              |
| Chocolate Platform     | S2S          | interstitial, rewarded, banner    | iOS, Android, Web     | App ID, API Key                     | 5s              |
| Tapdaq                 | S2S          | interstitial, rewarded, banner    | iOS, Android          | App ID, API Key                     | 5s              |
| Chartboost             | S2S          | interstitial, rewarded            | iOS, Android          | App ID, API Key                     | 5s              |
| Liftoff (Vungle)       | S2S          | interstitial, rewarded            | iOS, Android          | App ID, API Key                     | 5s              |
| Pangle (Bytedance)     | S2S          | interstitial, rewarded, banner    | iOS, Android          | App ID, API Key                     | 5s              |
| Meta Audience Network  | S2S          | interstitial, rewarded, banner    | iOS, Android          | App ID, App Secret                  | 5s              |
| Mintegral              | S2S          | interstitial, rewarded, banner    | iOS, Android          | App ID, API Key                     | 5s              |
| Moloco                 | S2S          | interstitial, rewarded            | iOS, Android          | Seat ID, API Key                    | 5s              |

Notes:
- All adapters adhere to the normalized NoBid taxonomy: timeout, network_error, status_XXX, no_fill, circuit_open, error.
- Conformance is verified in offline tests under `backend/auction/internal/bidders/*_conformance_test.go` and `adapter_conformance_test.go`.
