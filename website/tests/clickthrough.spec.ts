import { test, expect, Page } from '@playwright/test'

/**
 * Website "act like a user" click-through.
 *
 * This intentionally avoids cross-origin navigation (e.g. /signin redirects to the Console).
 * The goal is to click all visible, enabled buttons on every first-party marketing page.
 */

const PUBLIC_PAGES = [
  { path: '/', name: 'Home' },
  { path: '/product', name: 'Product' },
  { path: '/features', name: 'Features' },
  { path: '/pricing', name: 'Pricing' },
  { path: '/developers', name: 'Developers' },
  { path: '/security', name: 'Security' },
  { path: '/company', name: 'Company' },
  { path: '/contact', name: 'Contact' },
  { path: '/privacy', name: 'Privacy' },
  { path: '/terms', name: 'Terms' },
  { path: '/cookies', name: 'Cookies' },
  { path: '/gdpr', name: 'GDPR' },
  { path: '/dpa', name: 'DPA' },
  // Skip /signin because it intentionally redirects cross-origin to Console.
  { path: '/signup', name: 'Sign Up' },
]

type ClickOptions = {
  maxClicks?: number
  timeBudgetMs?: number
}

function isProbablyDestructive(label: string) {
  const s = label.toLowerCase()
  return s.includes('delete') || s.includes('remove') || s.includes('revoke') || s.includes('terminate') || s.includes('drop')
}

async function closeAnyModal(page: Page) {
  if (page.isClosed()) return
  await page.getByRole('button', { name: /close|dismiss|cancel|back/i }).first().click({ timeout: 750 }).catch(() => {})
  await page.keyboard.press('Escape').catch(() => {})
}

async function clickVisibleButtons(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 50
  const timeBudgetMs = options?.timeBudgetMs ?? 7_500
  const startedAt = Date.now()

  page.on('dialog', async (d) => {
    await d.dismiss().catch(() => {})
  })

  await page.waitForLoadState('domcontentloaded')

  const buttons = page.locator('button:not([disabled])')
  const count = await buttons.count().catch(() => 0)

  let clicks = 0
  for (let i = 0; i < count && clicks < maxClicks; i++) {
    if (Date.now() - startedAt > timeBudgetMs) break
    if (page.isClosed()) break

    const btn = buttons.nth(i)
    if (!(await btn.isVisible().catch(() => false))) continue

    const text = (await btn.textContent().catch(() => ''))?.trim()
    const ariaLabel = (await btn.getAttribute('aria-label').catch(() => null))
    const label = (text || ariaLabel || '').trim()

    if (label && isProbablyDestructive(label)) continue

    const typeAttr = (await btn.getAttribute('type').catch(() => null))
    if (typeAttr === 'submit') {
      // Skip submits in marketing pages (contact forms etc.) to keep runs side-effect free.
      continue
    }

    const clicked = await btn
      .click({ timeout: 750 })
      .then(() => true)
      .catch(() => false)

    if (!clicked) {
      await btn.scrollIntoViewIfNeeded().catch(() => {})
      await btn.click({ timeout: 750 }).catch(() => {})
    }

    clicks++
    if (clicks % 6 === 0) await closeAnyModal(page)
    await page.waitForTimeout(50).catch(() => {})
  }
}

test.describe('Website — click every button', () => {
  test.describe.configure({ timeout: 120_000 })

  for (const pageDef of PUBLIC_PAGES) {
    test(`${pageDef.name}: click visible buttons`, async ({ page }) => {
      const pageErrors: string[] = []
      const consoleErrors: string[] = []

      page.on('pageerror', (err) => {
        pageErrors.push(String(err))
      })
      page.on('console', (msg) => {
        if (msg.type() !== 'error') return
        const text = msg.text()
        if (text.includes('favicon')) return
        if (text.includes('Failed to load resource')) return
        consoleErrors.push(text)
      })

      const response = await page.goto(pageDef.path, { waitUntil: 'domcontentloaded' })
      expect(response?.status()).toBeGreaterThanOrEqual(200)
      expect(response?.status()).toBeLessThan(400)

      // Basic sanity: main content exists
      const mainLike = page.locator('main, [role="main"]').first()
      if ((await mainLike.count().catch(() => 0)) > 0) {
        await expect(mainLike).toBeVisible({ timeout: 10_000 })
      } else {
        await expect(page.locator('body')).toBeVisible({ timeout: 10_000 })
      }

      await clickVisibleButtons(page)

      expect(pageErrors, pageErrors.join('\n')).toHaveLength(0)
      expect(consoleErrors, consoleErrors.join('\n')).toHaveLength(0)
    })
  }
})
