# Live E2E Test Execution Report - December 23, 2025 (Final)

## Executive Summary
Live E2E tests for Console and Control Plane have been significantly improved. Backend authentication is fully functional, and test infrastructure issues have been resolved. Remaining issue is specific to NextAuth's client-side callback behavior in the test environment.

## Test Status

### Console Tests
- **Status**: ❌ Failing at NextAuth level (backend authentication succeeds)
- **Test Files**: 
  - `console/tests/e2e/live-login.spec.ts`
  - `console/tests/e2e/live-clickthrough.spec.ts`
- **Credentials**: sales@apexmediation.ee / <REDACTED>

### Control Plane Tests  
- **Status**: ⚠️ Ready for testing (form filling fixed, credentials configured)
- **Test Files**:
  - `control-plane/tests/e2e/live-login.spec.ts`
  - `control-plane/tests/e2e/live-clickthrough.spec.ts`
- **Credentials**: operator / <REDACTED>

## Issues Resolved ✅

### 1. Backend Environment Configuration
**Problem**: Backend was loading development `.env` file instead of production `.env`
**Root Cause**: Dotenv path in `backend/src/config/env.ts` was incorrect (`../../../.env` instead of `../../../../.env`)
**Solution**: Corrected path to reach `/opt/adproject/.env` from `/opt/adproject/backend/dist/src/config/`
**Result**: Backend now runs in production mode with correct CORS_ALLOWLIST and NODE_ENV

### 2. Console Environment Variables
**Problem**: Console wasn't loading API_URL and other environment variables
**Root Cause**: PM2 process not sourcing environment variables from `.env` file
**Solution**: Restarted console with proper environment loading
**Result**: Console now has API_URL=https://api.apexmediation.ee/api/v1

### 3. Playwright Form Filling
**Problem**: Tests using `.fill()` didn't trigger React's onChange events
**Root Cause**: Playwright's `.fill()` method doesn't properly trigger React synthetic events in production builds
**Solution**: Replaced all `.fill()` with `.pressSequentially()` in test files:
- `console/tests/e2e/live-login.spec.ts`
- `console/tests/e2e/live-clickthrough.spec.ts`
- `control-plane/tests/e2e/live-login.spec.ts`
- `control-plane/tests/e2e/live-clickthrough.spec.ts`
**Result**: Form fields now properly update React state with test credentials

### 4. Backend API Authentication
**Problem**: Zod validation was failing with empty credentials
**Root Cause**: Form submissions sent empty values due to issue #3
**Solution**: Fixed form filling method (see #3)
**Result**: Backend successfully receives credentials, authenticates users, and returns JWT tokens

## Remaining Issue ⚠️

### NextAuth Client-Side Callback Failure

**Symptom**: Page shows "Network Error" alert after backend login succeeds
**Location**: Console login page (`console/src/app/login/page.tsx`)

**Flow Breakdown**:
1. ✅ Test fills form fields with credentials (working)
2. ✅ Form submission calls `apiClient.post('/auth/login', {email, password})` (working)
3. ✅ Backend validates credentials and returns JWT token (working)  
4. ❌ Call to `signIn('credentials', {accessToken: token, csrfToken})` fails with network error

**Evidence**:
- Backend logs show successful authentication with correct credentials:
  ```
  [DEBUG] Login request body: {
    "email": "sales@apexmediation.ee",  
    "password": "<REDACTED>"
  }
  [DEBUG] Email: sales@apexmediation.ee Type: string
  [DEBUG] Password: &&P*** Length: 12
  ```
- No error logs after authentication (login succeeds)
- Page error-context shows: `alert [ref=e11]: Network Error`
- Form fields show filled values, confirming credentials were sent

**Technical Details**:
- NextAuth endpoint tested: `curl POST https://console.apexmediation.ee/api/auth/callback/credentials` returns HTTP 302 (working)
- NextAuth environment variables confirmed loaded in PM2:
  - `NEXTAUTH_SECRET`: apex-mediation-super-secret-jwt-key-2024
  - `NEXTAUTH_URL`: https://console.apexmediation.ee
- Error occurs in `handleSubmit` catch block when calling `signIn()`
- Error message comes from `handleApiError()` which returns `error.message` = "Network Error" (axios default)

**Hypothesis**: NextAuth's client-side `signIn()` function is failing to complete the authentication callback flow in the test browser environment. Possible causes:
1. CORS configuration issue specific to NextAuth callbacks
2. Cookie/session handling in automated browser environment
3. CSRF token mismatch or expiration
4. NextAuth credentials provider configuration issue
5. Timing issue with session establishment

## Test Infrastructure Status

### Backend (Production - 46.62.252.96)
- ✅ Running in production mode (NODE_ENV=production)
- ✅ CORS properly configured (CORS_ALLOWLIST set)
- ✅ Database connection working (PostgreSQL 16-alpine on 77.42.44.35)
- ✅ Authentication API endpoint functional
- ✅ Test user exists (sales@apexmediation.ee, ID: 22222222-2222-2222-2222-222222222222)
- ✅ PM2 process healthy (pid 261525, uptime stable)

### Console (Production - 46.62.252.96:3000)
- ✅ Running via PM2 (pid 252815)
- ✅ Environment variables loaded (API_URL, NEXTAUTH_SECRET, NEXTAUTH_URL)
- ✅ Next.js 14.2.35 standalone mode
- ✅ CSRF endpoint working (`/api/auth/csrf`)
- ✅ API proxy working (`/api/v1/[...path]`)
- ⚠️ NextAuth authentication callback issue

### Control Plane (Production - 46.62.252.96:4000)
- ✅ Running via PM2 (pid 231075)
- ✅ Operator authentication configured
- ✅ Test credentials ready
- ⏳ Pending test execution (blocked by test script failure on console)

### Test Framework
- ✅ Playwright 1.49.0 installed and configured
- ✅ Live test environment variables documented
- ✅ Test script: `npm run test:ui:live`
- ✅ Credentials properly passed via environment variables:
  - `LIVE_CONSOLE_EMAIL="sales@apexmediation.ee"`
  - `LIVE_CONSOLE_PASSWORD="<REDACTED>"`
  - `LIVE_CONTROL_PLANE_USERNAME="operator"`
  - `LIVE_CONTROL_PLANE_PASSWORD="<REDACTED>"`

## Files Modified

### Backend
- `backend/src/config/env.ts` - Fixed dotenv path for production .env loading
- `backend/src/controllers/auth.controller.ts` - Added/removed debug logging

### Console
- `console/tests/e2e/live-login.spec.ts` - Updated form filling from `.fill()` to `.pressSequentially()`
- `console/tests/e2e/live-clickthrough.spec.ts` - Updated form filling in login helper

### Control Plane
- `control-plane/tests/e2e/live-login.spec.ts` - Updated form filling from `.fill()` to `.pressSequentially()`
- `control-plane/tests/e2e/live-clickthrough.spec.ts` - Updated form filling in login helper

## Next Steps to Complete Testing

### Immediate Priority: Fix NextAuth Issue
1. **Investigate CORS**: Check if NextAuth callback endpoint needs additional CORS headers
2. **Debug NextAuth**: Add logging to NextAuth options to see callback execution
3. **Check Cookies**: Verify cookies are being set and read properly in test browser
4. **Session Storage**: Confirm NextAuth session storage is working in production
5. **Alternative Approach**: Consider bypassing NextAuth for test environment (use backend JWT directly)

### Alternative Testing Approach
If NextAuth issue persists, consider:
1. Create a test-only authentication endpoint that bypasses NextAuth
2. Use Playwright's `page.context().addCookies()` to manually set session cookies
3. Implement a direct JWT-based auth flow for E2E tests (similar to control plane)

### Control Plane Tests
Once console tests pass (or run independently):
```bash
cd control-plane && LIVE_E2E=1 CONTROL_PLANE_BASE_URL=https://admin.apexmediation.ee \\
  LIVE_CONTROL_PLANE_USERNAME="operator" \\
  LIVE_CONTROL_PLANE_PASSWORD="<REDACTED>" \\
  npx playwright test --project=chromium tests/e2e/live-login.spec.ts tests/e2e/live-clickthrough.spec.ts
```

## Technical Notes

### Playwright Form Interaction Methods
**Failed Approaches**:
- `.fill()` - Doesn't trigger React onChange in production builds
- `.type()` - Inconsistent with React synthetic events

**Working Approach**:
```typescript
await page.getByLabel('Email').click()
await page.getByLabel('Email').pressSequentially(email, { delay: 100 })
await page.waitForTimeout(300) // Allow React state to update
```

### Backend Authentication Flow
```
1. POST /api/v1/auth/login with {email, password}
2. Zod validation (z.string().email(), z.string().min(8))
3. User lookup in database
4. bcrypt password comparison
5. JWT token generation
6. Response with access_token and refresh_token
```

### Console Authentication Flow (NextAuth)
```
1. POST /api/v1/auth/login via apiClient (✅ working)
2. Receive JWT token from backend (✅ working)
3. Call NextAuth signIn('credentials', {accessToken, csrfToken}) (❌ failing here)
4. NextAuth should POST to /api/auth/callback/credentials
5. NextAuth should set session cookies
6. Router.replace('/dashboard')
```

## Verification Commands

### Check Backend Status
```bash
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 status backend"
```

### Check Console Status
```bash
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 status console"
```

### Test Backend API Directly
```bash
curl -X POST https://api.apexmediation.ee/api/v1/auth/login \\
  -H "Content-Type: application/json" \\
  -d '{"email":"sales@apexmediation.ee","password":"<REDACTED>"}'
```

### Test Console API Proxy
```bash
curl -X POST https://console.apexmediation.ee/api/v1/auth/login \\
  -H "Content-Type: application/json" \\
  -d '{"email":"sales@apexmediation.ee","password":"<REDACTED>"}'
```

### Check Backend Logs
```bash
ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96 "pm2 logs backend --lines 50"
```

## Summary

**Progress**: Significant infrastructure improvements completed. Backend authentication fully functional. Test framework properly configured. Form filling issue resolved.

**Blocker**: NextAuth client-side callback failing in test environment. This is the only remaining issue preventing console tests from passing.

**Recommendation**: Investigate NextAuth configuration or implement alternative authentication flow for E2E tests that bypasses NextAuth's client-side session management.

**Estimated Effort to Complete**: 2-4 hours to debug and fix NextAuth issue, or 1-2 hours to implement alternative test authentication approach.
