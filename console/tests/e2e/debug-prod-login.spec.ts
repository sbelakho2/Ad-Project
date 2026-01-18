import { test } from '@playwright/test'

// Opt-in local debug helper.
// To run:
//   CONSOLE_E2E_BASE_URL=https://console.example.com \
//   CONSOLE_E2E_EMAIL=you@example.com \
//   CONSOLE_E2E_PASSWORD=... \
//   npx playwright test console/tests/e2e/debug-prod-login.spec.ts
test.skip('Debug login and dashboard (opt-in)', async ({ page }) => {
  const baseUrl = process.env.CONSOLE_E2E_BASE_URL
  const email = process.env.CONSOLE_E2E_EMAIL
  const password = process.env.CONSOLE_E2E_PASSWORD

  if (!baseUrl || !email || !password) {
    throw new Error('Missing required env vars: CONSOLE_E2E_BASE_URL, CONSOLE_E2E_EMAIL, CONSOLE_E2E_PASSWORD')
  }

  await page.goto(`${baseUrl.replace(/\/+$/g, '')}/login`, { waitUntil: 'networkidle' })
  await page.waitForSelector('input[type="email"]', { timeout: 10000 })
  await page.fill('input[type="email"]', email)
  await page.fill('input[type="password"]', password)
  await page.click('button[type="submit"]')
  await page.waitForURL('**/dashboard', { timeout: 15000 })
})
