import { test, expect, Page } from '@playwright/test'

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
        snippet = text.slice(0, 1200)
      } catch {
        // ignore non-text bodies
      }

      serverErrors.push(`${status} ${url}${snippet ? `\n${snippet}` : ''}`)
    } catch {
      // best-effort only
    }
  })

  return serverErrors
}

function throwIfAnyServerErrors(serverErrors: string[], context: string) {
  if (serverErrors.length === 0) return
  throw new Error(`Server 5xx detected during ${context}:\n\n${serverErrors.join('\n\n')}`)
}

async function loginAsLiveOperator(page: Page) {
  const username = process.env.LIVE_CONTROL_PLANE_USERNAME
  const password = process.env.LIVE_CONTROL_PLANE_PASSWORD

  if (!username || !password) {
    throw new Error(
      'Missing LIVE_CONTROL_PLANE_USERNAME or LIVE_CONTROL_PLANE_PASSWORD.\n' +
        'These tests require REAL operator credentials.\n' +
        'See docs/LIVE_TESTING.md for setup instructions.'
    )
  }

  await page.goto('/admin/login', { waitUntil: 'domcontentloaded', timeout: 60_000 })

  // The control-plane login page is plain HTML; fill inputs directly.
  const usernameInput = page.locator('#username')
  const passwordInput = page.locator('#password')
  await expect(usernameInput).toBeVisible({ timeout: 30_000 })

  await usernameInput.fill(username)
  await passwordInput.fill(password)

  // Submit using the same JS logic as the existing live-login test.
  await page.evaluate(async () => {
    const username = (document.getElementById('username') as HTMLInputElement).value
    const password = (document.getElementById('password') as HTMLInputElement).value

    let deviceId = ''
    try {
      const existing = localStorage.getItem('controlPlaneDeviceId')
      if (existing && existing.length >= 16) {
        deviceId = existing
      } else {
        const bytes = new Uint8Array(32)
        if (window.crypto && window.crypto.getRandomValues) {
          window.crypto.getRandomValues(bytes)
        }
        const arr = Array.from(bytes)
        const bin = arr.map((b) => String.fromCharCode(b)).join('')
        deviceId = btoa(bin).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/g, '')
        localStorage.setItem('controlPlaneDeviceId', deviceId)
      }
    } catch {
      deviceId = 'test-device-id'
    }

    const deviceLabel = navigator.userAgent.slice(0, 120)

    const resp = await fetch('/api/v1/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password, deviceId, deviceLabel }),
    })

    if (!resp.ok) {
      const errorText = await resp.text()
      throw new Error(`Login failed: ${resp.status} - ${errorText}`)
    }

    const data = await resp.json()
    if (data.token) {
      localStorage.setItem('controlPlaneToken', data.token)
    }

    window.location.href = '/admin'
  })

  await page.waitForURL('**/admin', { timeout: 30_000 })
  await expect(page.locator('.brand', { hasText: 'APEX CONTROL PLANE' })).toBeVisible({ timeout: 30_000 })
  await expect(page.locator('#view-title')).toHaveText(/dashboard/i, { timeout: 30_000 })
}

test.describe('Control Plane LIVE — deep operator pages (non-destructive)', () => {
  test('Visit key operator pages and validate meaningful output', async ({ page, baseURL }) => {
    test.setTimeout(10 * 60 * 1000)

    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live tests')
    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')

    const origin = new URL(baseURL).origin
    const serverErrors = await startServerErrorCapture(page, origin)

    const pageErrors: string[] = []
    page.on('pageerror', (err) => {
      const where = (() => {
        try {
          return page.url()
        } catch {
          return '(unknown url)'
        }
      })()
      pageErrors.push(`[${where}] ${String(err)}`)
    })

    await loginAsLiveOperator(page)

    // Start from the main dashboard.
    await test.step('Visit /admin (dashboard)', async () => {
      await page.goto('/admin', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expect(page.locator('.brand', { hasText: 'APEX CONTROL PLANE' })).toBeVisible({ timeout: 30_000 })
      await expect(page.locator('.card-title', { hasText: /overall health/i })).toBeVisible({ timeout: 30_000 })
    })

    const adminLinks = page.locator('a[href^="/admin"]')
    const linkCount = await adminLinks.count().catch(() => 0)
    const hrefs: string[] = []
    for (let i = 0; i < linkCount; i++) {
      const a = adminLinks.nth(i)
      if (!(await a.isVisible().catch(() => false))) continue
      const href = (await a.getAttribute('href').catch(() => null)) || ''
      if (!href) continue
      if (href.includes('logout') || href.includes('signout')) continue
      if (!hrefs.includes(href)) hrefs.push(href)
    }

    for (const href of hrefs) {
      await test.step(`Visit ${href}`, async () => {
        let resp: any = null
        try {
          resp = await page.goto(href, { waitUntil: 'domcontentloaded', timeout: 60_000 })
        } catch (err) {
          const msg = String(err)
          if (!msg.includes('net::ERR_ABORTED')) throw err
          await page.waitForLoadState('domcontentloaded', { timeout: 60_000 }).catch(() => {})
        }

        // Must not silently return JSON Not Found.
        const status = resp?.status?.() ?? 0
        const bodyText = await page
          .locator('body')
          .innerText()
          .then((t) => t.replace(/\s+/g, ' ').trim())
          .catch(() => '')

        if (status === 404 || bodyText.includes('"error":"Not found"')) {
          throw new Error(`Route ${href} returned Not found: status=${status} body=${JSON.stringify(bodyText.slice(0, 240))}`)
        }

        await expect(page).not.toHaveURL(/\/admin\/login/i)
        await expect(page.getByText(/something went wrong|application error/i)).toHaveCount(0)

        // Expect some real UI content.
        await expect(page.getByRole('heading').first()).toBeVisible({ timeout: 30_000 })
      })
    }

    // Non-destructive interaction: click Refresh if present.
    await test.step('Click Refresh if present', async () => {
      const refresh = page.getByRole('button', { name: /^refresh$|reload/i })
      if (await refresh.first().isVisible().catch(() => false)) {
        await refresh.first().click().catch(() => {})
      }
    })

    // Keep this suite strict: page JS errors are real regressions here.
    if (pageErrors.length) {
      throw new Error(`Page JS errors detected (last ${Math.min(pageErrors.length, 10)}):\n\n${pageErrors.slice(-10).join('\n')}`)
    }

    throwIfAnyServerErrors(serverErrors, 'deep operator pages')
  })
})
