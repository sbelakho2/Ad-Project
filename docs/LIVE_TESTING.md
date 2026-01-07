# LIVE UI Testing Guide

## Overview
The repository includes LIVE end-to-end click-through tests that validate the Console and Control Plane UIs against the production domains:
- **Console**: `https://console.apexmediation.ee`
- **Control Plane**: `https://admin.apexmediation.ee`

These tests perform **real authentication** followed by exhaustive UI interaction (clicking buttons, links, navigating routes) while:
- Avoiding destructive actions (delete/remove/revoke)
- Avoiding side-effectful mutations (save/create/invite/pay)
- Allowing safe read-only interactions (search/filter/apply/refresh)
- Capturing any backend 5xx responses with URL + body snippet for debugging

## Prerequisites

### Console Tests
Requires a **real production user account**:
```bash
export LIVE_CONSOLE_EMAIL="sales@apexmediation.ee"
export LIVE_CONSOLE_PASSWORD='<from secret store>'
# Optional: if 2FA is enabled on the account
export LIVE_CONSOLE_2FA_CODE="123456"
```

**Available Test Account:**
- Email: `sales@apexmediation.ee`
- Password: (stored in the team secret store; never commit to repo)
- Note: Quote the password in shell due to special characters

### Control Plane Tests
Requires **real operator credentials**:
```bash
export LIVE_CONTROL_PLANE_USERNAME="operator"
export LIVE_CONTROL_PLANE_PASSWORD='<from secret store>'
```

**Available Operator Account:**
- Username: `operator`
- Password: (stored in the team secret store; never commit to repo)

## Running LIVE Tests

### Full Suite (Console + Control Plane)
```bash
LIVE_E2E=1 npm run test:ui
```

### Console Only
```bash
cd console
CONSOLE_BASE_URL=https://console.apexmediation.ee LIVE_E2E=1 npx playwright test --project=chromium tests/e2e/live-login.spec.ts tests/e2e/live-clickthrough.spec.ts
```

### Control Plane Only
```bash
cd control-plane
CONTROL_PLANE_BASE_URL=https://admin.apexmediation.ee LIVE_E2E=1 npx playwright test --project=chromium tests/e2e/live-login.spec.ts tests/e2e/live-clickthrough.spec.ts
```

## What Gets Tested

### Console LIVE Click-Through
After successful login, the test:
1. Visits ~25 customer-facing routes (dashboard, placements, analytics, settings, billing, etc.)
2. On each page:
   - Scrolls to reveal all UI
   - Clicks all visible enabled buttons (except destructive/side-effectful ones)
   - Clicks internal navigation links
   - Dismisses modals/dialogs
   - Collects runtime errors (page errors, console errors, 5xx responses)
3. Fails with actionable diagnostics if any 5xx response is detected

### Control Plane LIVE Click-Through
After successful operator login, the test:
1. Visits `/admin` dashboard
2. Clicks all visible enabled buttons/controls (except dangerous `.btn-danger` and destructive actions)
3. Follows internal `/admin/*` navigation links
4. Collects runtime errors and 5xx responses

## Diagnostics & Debugging

### If Login Fails with 400
- The credentials are invalid or the user doesn't exist
- **Most common cause**: The backend `/api/v1/auth/login` endpoint is rejecting the credentials
- Check that the email/password match a real production user account
- Verify the account is active and not locked
- Try logging in manually via browser to confirm credentials work
- Check backend logs for the actual rejection reason
- Verify 2FA code if 2FA is enabled (check for `2FA required` alert in error screenshots)

**If using the provided test account (`sales@apexmediation.ee`) and still getting 400:**
1. Verify the account exists in production database:
   ```sql
   SELECT id, email, created_at FROM users WHERE email = 'sales@apexmediation.ee';
   ```
2. Check if password hash matches:
   ```sql
   SELECT password_hash FROM users WHERE email = 'sales@apexmediation.ee';
   ```
3. Ensure backend `/api/v1/auth/login` is accessible:
   ```bash
   curl -X POST https://api.apexmediation.ee/api/v1/auth/login \
     -H "Content-Type: application/json" \
       -d '{"email":"sales@apexmediation.ee","password":"'"$LIVE_CONSOLE_PASSWORD"'"}'
   ```

### If Login Fails with 500
- The test will fail immediately with the exact 5xx URL and response body snippet
- Example output:
  ```
  Server 5xx detected during primary sign-in:

  500 https://console.apexmediation.ee/api/auth/callback/credentials
  {"error":"Internal Server Error","message":"Database connection failed"}
  ```
- Use this to pinpoint the failing backend endpoint/service

### If Tests Timeout Without 5xx
- The page may be stuck on `/login` due to frontend issue (check screenshots in `test-results/`)
- Or a redirect loop / SPA navigation issue (check console logs in error context)

## Safety Guarantees
- Tests **never** click: delete, remove, revoke, terminate, drop, reset, destroy, wipe, logout, save, create, add, invite, send, purchase, upgrade, pay, generate
- Tests **only** click safe submits: search, filter, apply, refresh, reload, test, validate, preview
- Tests run in isolated Chromium contexts with no shared state between runs
- All tests are explicitly opt-in via `LIVE_E2E=1` and require credentials from environment (never hardcoded)

## CI/CD Integration
To run LIVE tests in CI:
1. Store credentials as **secrets** (never commit to repo)
2. Set env vars in CI job:
   ```yaml
   - name: Run LIVE UI Tests
     env:
       LIVE_E2E: "1"
       LIVE_CONSOLE_EMAIL: ${{ secrets.LIVE_CONSOLE_EMAIL }}
       LIVE_CONSOLE_PASSWORD: ${{ secrets.LIVE_CONSOLE_PASSWORD }}
       LIVE_CONTROL_PLANE_USERNAME: ${{ secrets.LIVE_CONTROL_PLANE_USERNAME }}
       LIVE_CONTROL_PLANE_PASSWORD: ${{ secrets.LIVE_CONTROL_PLANE_PASSWORD }}
     run: npm run test:ui
   ```
3. Review Playwright HTML report artifact for screenshots/traces on failure

## Notes
- These tests use **production data** and **real authentication**
- Do NOT run against staging or dev with production credentials
- If a test fails, always check the error context markdown and screenshot artifacts in `test-results/`
- The tests are designed to be **non-destructive** but they do generate API traffic (reads, searches, filters)
