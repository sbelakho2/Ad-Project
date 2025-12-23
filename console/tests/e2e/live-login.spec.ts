import { test, expect } from '@playwright/test'

test.describe('Console LIVE — login with dummy user', () => {
  test('login and reach dashboard', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live login tests')

    const email = process.env.LIVE_CONSOLE_EMAIL
    const password = process.env.LIVE_CONSOLE_PASSWORD
    const twofa = process.env.LIVE_CONSOLE_2FA_CODE

    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')
    if (!email || !password) {
      throw new Error('Missing LIVE_CONSOLE_EMAIL or LIVE_CONSOLE_PASSWORD')
    }

    await page.goto('/login', { waitUntil: 'domcontentloaded' })

    await page.getByLabel('Email').fill(email)
    await page.getByLabel('Password').fill(password)
    await page.getByRole('button', { name: /^sign in$/i }).click()

    const twofaAlert = page.getByRole('alert').filter({ hasText: /Two-factor verification required/i })
    if (await twofaAlert.isVisible().catch(() => false)) {
      if (!twofa) {
        throw new Error('2FA is required for this dummy user. Set LIVE_CONSOLE_2FA_CODE and re-run.')
      }
      await page.getByLabel(/2FA Code or Backup Code/i).fill(twofa)
      await page.getByRole('button', { name: /verify and continue/i }).click()
    }

    await page.waitForURL('**/dashboard**', { timeout: 45_000 })
    await expect(page).not.toHaveURL(/\/login(\?|$)/)

    // Sanity: app shell renders
    await expect(page.getByRole('navigation', { name: /main navigation/i })).toBeVisible({ timeout: 15_000 })
  })
})
