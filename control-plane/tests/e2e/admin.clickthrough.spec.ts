import { test, expect } from '@playwright/test'

function isProbablyDestructive(label: string) {
  const s = label.toLowerCase()
  return s.includes('delete') || s.includes('drop') || s.includes('terminate') || s.includes('reset')
}

test.describe('Control Plane operator UI — click-through', () => {
  test('sign in and click every button/link (safe, mocked APIs)', async ({ page }) => {
    const pageErrors: string[] = []
    const consoleErrors: string[] = []
    page.on('pageerror', (err) => pageErrors.push(String(err)))
    page.on('console', (msg) => {
      if (msg.type() === 'error') consoleErrors.push(msg.text())
    })

    // Detect whether the login page JS attaches a submit handler.
    await page.addInitScript(() => {
      ;(window as any).__apexLoginSubmitHooked = false
      const orig = EventTarget.prototype.addEventListener
      EventTarget.prototype.addEventListener = function (type: any, listener: any, options: any) {
        try {
          if (type === 'submit' && (this as any)?.id === 'login-form') {
            ;(window as any).__apexLoginSubmitHooked = true
          }
        } catch {
          // ignore
        }
        return orig.call(this, type, listener, options)
      }
    })

    // Mock dashboard + device APIs so we can exercise UI without a real DB.
    // Keep login API real.
    const now = new Date().toISOString()

    let deviceRevoked = false

    await page.route('**/api/v1/dashboard', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ok: true,
          service: 'control-plane',
          timestamp: now,
          note: 'mocked for playwright clickthrough',
        }),
      })
    })

    await page.route('**/api/v1/devices', async (route) => {
      const device = {
        id: 'dev-1',
        deviceLabel: 'Playwright Mac',
        userAgent: 'Playwright',
        addedAt: now,
        lastSeenAt: now,
        revokedAt: deviceRevoked ? now : null,
      }
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ devices: [device] }),
      })
    })

    await page.route('**/api/v1/devices/*/revoke', async (route) => {
      deviceRevoked = true
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ok: true }),
      })
    })

    // Click-through should accept confirm dialogs (Revoke uses window.confirm).
    page.on('dialog', async (d) => {
      await d.accept().catch(() => {})
    })

    // Sign in via the real operator login form.
    const loginScriptLoaded = page.waitForResponse((r) =>
      r.url().includes('/static/admin-login.js') && r.status() === 200
    )

    await page.goto('/admin/login', { waitUntil: 'domcontentloaded' })
    await loginScriptLoaded

    const hooked = await page
      .waitForFunction(() => (window as any).__apexLoginSubmitHooked === true, null, { timeout: 2000 })
      .then(() => true)
      .catch(() => false)

    if (!hooked) {
      const diag = await page.evaluate(() => {
        const scripts = Array.from(document.scripts).map((s) => ({ src: s.src || null, defer: (s as any).defer || false }))
        return {
          hasLoginForm: Boolean(document.getElementById('login-form')),
          hasUsername: Boolean(document.getElementById('username')),
          hasPassword: Boolean(document.getElementById('password')),
          hasErrorDiv: Boolean(document.getElementById('error')),
          hasSubmitId: Boolean(document.getElementById('submit')),
          scriptTags: scripts,
          location: window.location.href,
        }
      })

      throw new Error(
        `Login submit handler did not attach. diag=${JSON.stringify(diag)}\n` +
          `pageErrors=${pageErrors.join('\n')}\nconsoleErrors=${consoleErrors.join('\n')}`
      )
    }
    await expect(page.getByRole('heading', { name: /operator access/i })).toBeVisible()

    await page.locator('#username').fill('playwright@local.test')
    await page.locator('#password').fill('playwright-pass')

    await page.getByRole('button', { name: /sign in/i }).click()

    await expect(page).toHaveURL(/\/admin\/?(\?|$)/)
    await expect(page.getByRole('heading', { name: /control plane/i })).toBeVisible()

    // If the login JS crashed, fail with the error details.
    expect(
      pageErrors,
      `pageerrors:\n${pageErrors.join('\n')}\nconsoleErrors:\n${consoleErrors.join('\n')}`
    ).toEqual([])

    // Wait for the devices table to render (driven by JS).
    await expect(page.getByRole('heading', { name: /trusted devices/i })).toBeVisible()

    // Click all visible buttons first (includes Revoke).
    const buttons = page.locator('button')
    const buttonCount = await buttons.count()
    for (let i = 0; i < buttonCount; i++) {
      const btn = buttons.nth(i)
      if (!(await btn.isVisible().catch(() => false))) continue
      if (!(await btn.isEnabled().catch(() => false))) continue

      const label = ((await btn.textContent().catch(() => '')) || '').trim()
      if (label && isProbablyDestructive(label)) continue

      await btn.click({ timeout: 2000 }).catch(() => {})
    }

    // Click links, but do Sign out last.
    const links = page.locator('a')
    const linkCount = await links.count()

    const signOutCandidates: number[] = []
    for (let i = 0; i < linkCount; i++) {
      const a = links.nth(i)
      if (!(await a.isVisible().catch(() => false))) continue

      const text = ((await a.textContent().catch(() => '')) || '').trim()
      const href = (await a.getAttribute('href').catch(() => null)) || ''
      if (!href || href.startsWith('http') || href.startsWith('mailto:') || href.startsWith('#')) continue

      if (/sign out/i.test(text) || /\/admin\/logout/.test(href)) {
        signOutCandidates.push(i)
        continue
      }

      await a.click({ timeout: 2000 }).catch(() => {})
    }

    // Finally, sign out.
    if (signOutCandidates.length > 0) {
      const a = links.nth(signOutCandidates[0])
      await a.click({ timeout: 2000 }).catch(() => {})
      await expect(page).toHaveURL(/\/admin\/login/)
    }

    // Sanity: login page should still render.
    await expect(page.getByRole('button', { name: /sign in/i })).toBeVisible()
  })
})
