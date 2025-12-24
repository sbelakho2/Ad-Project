import { test, expect, Page } from '@playwright/test'

type ClickOptions = {
  maxClicks?: number
  timeBudgetMs?: number
  rounds?: number
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
    s.includes('reset') ||
    s.includes('destroy') ||
    s.includes('wipe') ||
    s.includes('sign out') ||
    s.includes('log out') ||
    s === 'logout'
  )
}

function isProbablySideEffectful(label: string) {
  const s = label.toLowerCase()
  return (
    s.includes('save') ||
    s.includes('create') ||
    s.includes('add ') ||
    s.startsWith('add') ||
    s.includes('invite') ||
    s.includes('send') ||
    s.includes('submit') ||
    s.includes('purchase') ||
    s.includes('upgrade') ||
    s.includes('pay') ||
    s.includes('generate')
  )
}

function isSafeSubmit(label: string) {
  const s = label.toLowerCase()
  return (
    s.includes('search') ||
    s.includes('filter') ||
    s.includes('apply') ||
    s.includes('refresh') ||
    s.includes('reload') ||
    s.includes('test') ||
    s.includes('validate') ||
    s.includes('preview')
  )
}

async function closeAnyModal(page: Page) {
  if (page.isClosed()) return
  await page.getByRole('button', { name: /cancel|close|dismiss|not now|back/i }).first().click({ timeout: 750 }).catch(() => {})
  await page.keyboard.press('Escape').catch(() => {})
}

async function clickVisibleButtons(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 40
  const timeBudgetMs = options?.timeBudgetMs ?? 7_500
  const rounds = options?.rounds ?? 2
  const startedAt = Date.now()

  page.on('dialog', async (d) => {
    await d.dismiss().catch(() => {})
  })

  await page.waitForLoadState('domcontentloaded')

  const buttons = page.locator(
    [
      'button:not([disabled])',
      '[role="button"]:not([aria-disabled="true"])',
      '[role="menuitem"]:not([aria-disabled="true"])',
    ].join(', ')
  )

  let clicks = 0
  for (let round = 0; round < rounds && clicks < maxClicks; round++) {
    await page.waitForLoadState('domcontentloaded')
    await page.waitForTimeout(100).catch(() => {})

    // Scroll to reveal more content/buttons.
    await page.evaluate(() => window.scrollTo(0, 0)).catch(() => {})
    await page.waitForTimeout(50).catch(() => {})
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight)).catch(() => {})
    await page.waitForTimeout(100).catch(() => {})
    await page.evaluate(() => window.scrollTo(0, 0)).catch(() => {})

    const count = await buttons.count().catch(() => 0)

    for (let i = 0; i < count && clicks < maxClicks; i++) {
      if (Date.now() - startedAt > timeBudgetMs) break
      if (page.isClosed()) break

      const btn = buttons.nth(i)
      if (!(await btn.isVisible().catch(() => false))) continue

      const text = (await btn.textContent().catch(() => ''))?.trim()
      const ariaLabel = (await btn.getAttribute('aria-label').catch(() => null))
      const label = (text || ariaLabel || '').trim()

      if (label && isProbablyDestructive(label)) continue

      if (label && isProbablySideEffectful(label)) continue

      const typeAttr = (await btn.getAttribute('type').catch(() => null))
      if (typeAttr === 'submit') {
        // Allow only obviously safe submits (search/filter/apply).
        if (!label || !isSafeSubmit(label)) continue
      }

      await btn.scrollIntoViewIfNeeded().catch(() => {})
      await btn.click({ timeout: 750 }).catch(() => {})

      clicks++
      if (clicks % 6 === 0) await closeAnyModal(page)
      await page.waitForTimeout(50).catch(() => {})
    }
  }
}

async function startServerErrorCapture(page: Page, origin: string) {
  const serverErrors: string[] = []

  page.on('response', async (res) => {
    try {
      const status = res.status()
      if (status < 500) return
      const url = res.url()
      if (!url.startsWith(origin)) return

      let snippet = ''
      try {
        const text = await res.text()
        snippet = text.slice(0, 2000)
      } catch {
        // ignore non-text bodies
      }

      serverErrors.push(`${status} ${url}${snippet ? `\n${snippet}` : ''}`)
    } catch {
      // best-effort capture only
    }
  })

  return serverErrors
}

function isProbablyExternal(href: string) {
  return href.startsWith('http://') || href.startsWith('https://') || href.startsWith('mailto:') || href.startsWith('tel:')
}

async function clickInternalLinks(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 25
  const timeBudgetMs = options?.timeBudgetMs ?? 6_000
  const startedAt = Date.now()

  const links = page.locator('a[href]')
  const count = await links.count().catch(() => 0)

  let clicks = 0
  for (let i = 0; i < count && clicks < maxClicks; i++) {
    if (Date.now() - startedAt > timeBudgetMs) break
    if (page.isClosed()) break

    const link = links.nth(i)
    if (!(await link.isVisible().catch(() => false))) continue

    const href = (await link.getAttribute('href').catch(() => null))
    if (!href) continue
    if (isProbablyExternal(href)) continue
    if (!href.startsWith('/')) continue
    if (href.startsWith('/logout') || href.includes('signout') || href.includes('logout')) continue

    await link.scrollIntoViewIfNeeded().catch(() => {})
    await link.click({ timeout: 750 }).catch(() => {})
    await page.waitForLoadState('domcontentloaded').catch(() => {})
    clicks++
  }
}

async function liveLogin(page: Page) {
  const email = process.env.LIVE_CONSOLE_EMAIL
  const password = process.env.LIVE_CONSOLE_PASSWORD
  const twofa = process.env.LIVE_CONSOLE_2FA_CODE

  if (!email || !password) {
    throw new Error(
      'Missing LIVE_CONSOLE_EMAIL or LIVE_CONSOLE_PASSWORD.\n' +
      'These tests require REAL production credentials.\n' +
      'See docs/LIVE_TESTING.md for setup instructions.'
    )
  }

  await page.goto('/login', { waitUntil: 'domcontentloaded' })

  // Use pressSequentially() for better React compatibility
  await page.getByLabel('Email').click()
  await page.getByLabel('Email').pressSequentially(email, { delay: 100 })
  
  await page.getByLabel('Password').click()
  await page.getByLabel('Password').pressSequentially(password, { delay: 100 })
  
  // Wait a moment for React state to update
  await page.waitForTimeout(300)
  
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
}

test.describe('Console LIVE — click everything (dummy user)', () => {
  test.describe.configure({ timeout: 240_000 })

  test('login then click through all customer surfaces', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live click-through')

    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')
    const origin = new URL(baseURL).origin

    const pageErrors: string[] = []
    const consoleErrors: string[] = []

    page.on('pageerror', (err) => pageErrors.push(String(err)))
    page.on('console', (msg) => {
      if (msg.type() !== 'error') return
      const text = msg.text()
      if (text.includes('Failed to load resource')) return
      if (text.includes('favicon')) return
      if (text.includes('Failed to fetch RSC payload')) return
      consoleErrors.push(text)
    })

    const serverErrors = await startServerErrorCapture(page, origin)

    await liveLogin(page)

    // Fail fast if login triggered a backend 5xx.
    expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)

    const routes = [
      '/dashboard',
      '/placements',
      '/adapters',
      '/analytics',
      '/fraud',
      '/supply-chain',
      '/tools/supply-chain-status',
      '/tools/app-ads-inspector',
      '/tools/mediation-debugger',
      '/recon/overview',
      '/recon/deltas',
      '/transparency/summary',
      '/transparency/auctions',
      '/settings',
      '/settings/keys',
      '/settings/sdk',
      '/settings/security',
      '/settings/compliance',
      '/settings/team',
      '/settings/notifications',
      '/settings/networks',
      '/settings/credentials',
      '/settings/billing/settings',
      '/settings/billing/usage',
      '/settings/billing/invoices',
      '/migration-studio',
    ]

    for (const route of routes) {
      await page.goto(route, { waitUntil: 'domcontentloaded' })
      await expect(page).not.toHaveURL(/\/login|\/signin|\/auth\//)

      await clickVisibleButtons(page, { rounds: 3, maxClicks: 60, timeBudgetMs: 12_000 })
      await clickInternalLinks(page, { maxClicks: 15, timeBudgetMs: 4_000 })
      await closeAnyModal(page)

      // Stop early with a useful error if any route triggers 5xx.
      expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)
    }

    expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)
    expect(pageErrors, pageErrors.join('\n')).toHaveLength(0)
    expect(consoleErrors, consoleErrors.join('\n')).toHaveLength(0)
  })
})
