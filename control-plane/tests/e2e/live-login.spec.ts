import { test, expect } from '@playwright/test'

test.describe('Control Plane LIVE — login with dummy operator', () => {
  test('login and reach /admin dashboard', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live login tests')

    const username = process.env.LIVE_CONTROL_PLANE_USERNAME
    const password = process.env.LIVE_CONTROL_PLANE_PASSWORD

    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')
    if (!username || !password) {
      throw new Error('Missing LIVE_CONTROL_PLANE_USERNAME or LIVE_CONTROL_PLANE_PASSWORD')
    }

    await page.goto('/admin/login', { waitUntil: 'domcontentloaded' })

    await page.getByLabel(/username \/ email/i).fill(username)
    await page.getByLabel(/^password$/i).fill(password)
    await page.getByRole('button', { name: /^sign in$/i }).click()

    await page.waitForURL('**/admin', { timeout: 30_000 })
    await expect(page.getByRole('heading', { name: /^Control Plane$/i })).toBeVisible({ timeout: 10_000 })
  })
})
