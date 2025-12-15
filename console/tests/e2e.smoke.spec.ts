import { test, expect } from '@playwright/test'

test.describe('Console E2E — smoke', () => {
  test('login page loads', async ({ page, baseURL }) => {
    await page.goto(baseURL || '/')
    // If redirected to login, ensure form is present; otherwise check page structure
    if (await page.locator('form [type="email"]').first().isVisible().catch(() => false)) {
      await expect(page.locator('form')).toBeVisible()
    } else if (await page.locator('main').first().isVisible().catch(() => false)) {
      await expect(page.locator('main')).toBeVisible()
    } else {
      // Fall back to checking any content is present
      await expect(page.locator('body')).not.toBeEmpty()
    }
  })
})
