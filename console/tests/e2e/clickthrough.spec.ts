import { test, expect, Page } from '@playwright/test'

/**
 * Click-through “act like a user” suite.
 *
 * Goal: visit key pages while authenticated (mock-mode), and click every visible, enabled button/link
 * in a safe way (avoid destructive/irreversible actions).
 */

type ClickOptions = {
  maxClicks?: number
  timeBudgetMs?: number
}

function isLoginLike(url: string) {
  try {
    const u = new URL(url)
    return /\/login|\/signin|\/auth\//.test(u.pathname)
  } catch {
    return /\/login|\/signin|\/auth\//.test(url)
  }
}

function isProbablyDestructive(label: string) {
  const s = label.toLowerCase()
  return (
    s.includes('delete') ||
    s.includes('remove') ||
    s.includes('revoke') ||
    s.includes('disable') ||
    s.includes('terminate') ||
    s.includes('drop') ||
    s.includes('reset')
  )
}

async function closeAnyModal(page: Page) {
  if (page.isClosed()) return

  // Common close/cancel affordances across the app
  const candidates = [
    page.getByRole('button', { name: /cancel|close|dismiss|not now|back/i }),
    page.locator('[data-testid="close"], [data-testid="modal-close"], [aria-label="Close"], [aria-label="Close dialog"]'),
  ]

  for (const locator of candidates) {
    const count = await locator.count()
    if (!count) continue
    for (let i = 0; i < count; i++) {
      const el = locator.nth(i)
      if (await el.isVisible().catch(() => false)) {
        await el.click({ timeout: 1000 }).catch(() => {})
      }
    }
  }

  // Try ESC as a last resort
  await page.keyboard.press('Escape').catch(() => {})
}

async function clickVisibleInteractives(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 80
  const timeBudgetMs = options?.timeBudgetMs ?? 25_000
  const startedAt = Date.now()

  page.on('dialog', async (d) => {
    await d.dismiss().catch(() => {})
  })

  // Ensure baseline content is present
  await page.waitForLoadState('domcontentloaded')

  // Collect interactives in DOM order (buttons). We intentionally avoid clicking links
  // since this suite already navigates across the main routes and link-clicking tends to
  // explode runtime via navigation/back loops.
  // Avoid ':visible' here; it can be very expensive on complex pages.
  const buttonLocator = page.locator('button:not([disabled])')

  let clicks = 0

  // Buttons
  const buttonCount = await buttonLocator.count()
  for (let i = 0; i < buttonCount && clicks < maxClicks; i++) {
    if (Date.now() - startedAt > timeBudgetMs) break
    if (page.isClosed() || isLoginLike(page.url())) break
    const btn = buttonLocator.nth(i)

    const visible = await btn.isVisible().catch(() => false)
    if (!visible) continue

    const text = (await btn.textContent().catch(() => ''))?.trim()
    const ariaLabel = (await btn.getAttribute('aria-label').catch(() => null))
    const label = (text || ariaLabel || '').trim()
    if (label && isProbablyDestructive(label)) continue

    // Avoid file inputs masquerading as buttons
    const typeAttr = (await btn.getAttribute('type').catch(() => null))
    if (typeAttr === 'submit') {
      // Only submit if it looks like an in-page safe action; otherwise skip.
      if (label && /save|apply|update|filter|search|export|refresh/i.test(label)) {
        await btn.click({ timeout: 1000 }).catch(() => {})
        clicks++
        await closeAnyModal(page)
      }
      continue
    }

    // Avoid forcing layout for every element; only scroll on failure.
    const clicked = await btn
      .click({ timeout: 750 })
      .then(() => true)
      .catch(() => false)

    if (!clicked) {
      await btn.scrollIntoViewIfNeeded().catch(() => {})
      await btn.click({ timeout: 750 }).catch(() => {})
    }
    clicks++
    await page.waitForTimeout(50).catch(() => {})

    // Closing a modal is expensive; do it periodically.
    if (clicks % 5 === 0) {
      await closeAnyModal(page)
    }
  }
}

const PAGES = [
  '/dashboard',
  '/analytics',
  '/placements',
  '/adapters',
  '/billing/usage',
  '/settings',
  '/settings/keys',
  '/settings/security',
  '/migration-studio',
]

test.describe('Console E2E — click-through', () => {
  test.describe.configure({ timeout: 120_000 })

  for (const path of PAGES) {
    test(`click through ${path}`, async ({ page }) => {
      await page.addInitScript(() => {
        ;(window as any).__APEX_E2E_SESSION__ = {
          userId: 'usr-playwright',
          publisherId: 'pub-test-123',
          email: 'playwright@example.com',
          role: 'admin',
          permissions: ['billing:view', 'billing:manage'],
        }
      })

      await page.goto(path, { waitUntil: 'domcontentloaded' })

      // Basic sanity: page content exists and we are not on login.
      await expect(page.locator('body')).toBeVisible()
      await expect(page).not.toHaveURL(/\/login|\/signin|\/auth\//)

      await clickVisibleInteractives(page)

      // Ensure we didn't trigger the global error page
      await expect(page.getByRole('alert').filter({ hasText: /Something went wrong/i })).toHaveCount(0, {
        timeout: 1000,
      })
    })
  }
})
