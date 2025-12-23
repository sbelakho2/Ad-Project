import { test, expect, Page } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'

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
    s.includes('reset') ||
    s.includes('destroy') ||
    s.includes('wipe') ||
    s.includes('sign out') ||
    s.includes('log out') ||
    s === 'logout'
  )
}

function isSkippableSegment(seg: string) {
  // Next.js route groups and dynamic segments are not directly navigable
  if (!seg) return true
  if (seg.startsWith('(') && seg.endsWith(')')) return true
  if (seg.startsWith('[') && seg.endsWith(']')) return true
  if (seg.startsWith('_')) return true
  if (seg === 'api') return true
  return false
}

function collectStaticAppRoutes(appDir: string): string[] {
  const routes = new Set<string>()

  const walk = (dir: string) => {
    const entries = fs.readdirSync(dir, { withFileTypes: true })
    for (const entry of entries) {
      const abs = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        // Skip node_modules-style dirs and Next special dirs
        if (entry.name === 'node_modules' || entry.name === '.next') continue
        walk(abs)
        continue
      }

      if (!entry.isFile()) continue
      if (!/^page\.(t|j)sx?$/.test(entry.name)) continue

      const rel = path.relative(appDir, abs)
      const parts = rel.split(path.sep)

      // Drop trailing "page.tsx"
      parts.pop()

      // Ignore anything with dynamic segments or route groups
      if (parts.some((p) => p.includes('[') || p.includes(']') || (p.startsWith('(') && p.endsWith(')')))) continue
      if (parts.some(isSkippableSegment)) continue

      const route = '/' + parts.filter(Boolean).join('/')
      routes.add(route === '/' ? '/' : route)
    }
  }

  walk(appDir)

  // The Console app uses '/' as a special landing; tests generally start from /dashboard
  routes.delete('/')

  // Skip dedicated auth/operator entrypoints and other non-customer surfaces.
  // These either intentionally render auth UIs or redirect cross-origin to the Control Plane.
  for (const r of Array.from(routes)) {
    if (/^\/(login|signup)(\/|$)/.test(r)) routes.delete(r)
    if (/^\/admin(\/|$)/.test(r)) routes.delete(r)
    if (r === '/403') routes.delete(r)
  }

  return Array.from(routes).sort()
}

async function closeAnyModal(page: Page) {
  if (page.isClosed()) return

  const candidates = [
    page.getByRole('button', { name: /cancel|close|dismiss|not now|back/i }),
    page.locator('[data-testid="close"], [data-testid="modal-close"], [aria-label="Close"], [aria-label="Close dialog"]'),
  ]

  for (const locator of candidates) {
    const count = await locator.count().catch(() => 0)
    if (!count) continue
    for (let i = 0; i < count; i++) {
      const el = locator.nth(i)
      if (await el.isVisible().catch(() => false)) {
        await el.click({ timeout: 1000 }).catch(() => {})
      }
    }
  }

  await page.keyboard.press('Escape').catch(() => {})
}

async function clickVisibleButtons(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 35
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
    if (page.isClosed() || isLoginLike(page.url())) break

    const btn = buttons.nth(i)
    if (!(await btn.isVisible().catch(() => false))) continue

    const text = (await btn.textContent().catch(() => ''))?.trim()
    const ariaLabel = (await btn.getAttribute('aria-label').catch(() => null))
    const label = (text || ariaLabel || '').trim()

    if (label && isProbablyDestructive(label)) continue

    const typeAttr = (await btn.getAttribute('type').catch(() => null))
    if (typeAttr === 'submit') {
      if (label && /save|apply|update|filter|search|export|refresh/i.test(label)) {
        await btn.click({ timeout: 1000 }).catch(() => {})
        clicks++
        await closeAnyModal(page)
      }
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
    if (clicks % 5 === 0) await closeAnyModal(page)
    await page.waitForTimeout(50).catch(() => {})
  }
}

test.describe('Console E2E — deep click-through (all static routes)', () => {
  test.describe.configure({ timeout: 180_000 })

  test('visit every static route and click every safe button', async ({ page }) => {
    await page.addInitScript(() => {
      ;(window as any).__APEX_E2E_SESSION__ = {
        userId: 'usr-playwright',
        publisherId: 'pub-test-123',
        email: 'playwright@example.com',
        role: 'admin',
        permissions: ['billing:view', 'billing:manage'],
      }
    })

    // Resolve relative to this file so it works regardless of Playwright cwd.
    const appDir = path.resolve(__dirname, '../../src/app')
    const routes = collectStaticAppRoutes(appDir)

    // If this ever becomes huge, it’s a sign the suite should be split, but for now keep it single-test.
    expect(routes.length).toBeGreaterThan(0)

    const pageErrors: string[] = []
    const consoleErrors: string[] = []

    page.on('pageerror', (err) => {
      pageErrors.push(String(err))
    })
    page.on('console', (msg) => {
      if (msg.type() !== 'error') return
      const text = msg.text()
      // Common non-actionable noise in mock/dev mode.
      if (text.includes('Failed to load resource')) return
      if (text.includes('favicon')) return
      if (text.includes('Failed to fetch RSC payload')) return
      if (text.includes('[next-auth]') && text.includes('CLIENT_FETCH_ERROR')) return
      consoleErrors.push(text)
    })

    for (const route of routes) {
      await page.goto(route, { waitUntil: 'domcontentloaded' })

      // If a page redirects to login, that’s a real regression for mock-mode.
      await expect(page).not.toHaveURL(/\/login|\/signin|\/auth\//)

      // Skip hard 404 pages (shouldn’t happen for static routes; if it does, we want a clear failure).
      await expect(page.getByRole('heading', { name: /^404$/ })).toHaveCount(0)

      await clickVisibleButtons(page)

      // Best-effort: avoid accumulating overlays between pages.
      await closeAnyModal(page)
    }

    expect(pageErrors, pageErrors.join('\n')).toHaveLength(0)
    expect(consoleErrors, consoleErrors.join('\n')).toHaveLength(0)
  })
})
