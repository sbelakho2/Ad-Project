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
    s.includes('approve') ||
    s.includes('deny')
  )
}

function isSafeSubmit(label: string) {
  const s = label.toLowerCase()
  return s.includes('search') || s.includes('filter') || s.includes('apply') || s.includes('refresh') || s.includes('reload')
}

async function closeAnyModal(page: Page) {
  if (page.isClosed()) return
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

  // Control Plane UI uses .btn and .btn-danger; avoid danger actions.
  const buttons = page.locator('button.btn:not([disabled]):not(.btn-danger), button:not([disabled]), [role="button"]:not([aria-disabled="true"])')

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

      const text = (await btn.textContent().catch(() => ''))?.trim() || ''
      const ariaLabel = (await btn.getAttribute('aria-label').catch(() => null))
      const label = (text || ariaLabel || '').trim()

      if (label && isProbablyDestructive(label)) continue
      if (label && isProbablySideEffectful(label)) continue

      const typeAttr = (await btn.getAttribute('type').catch(() => null))
      if (typeAttr === 'submit') {
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

async function liveLogin(page: Page) {
  const username = process.env.LIVE_CONTROL_PLANE_USERNAME
  const password = process.env.LIVE_CONTROL_PLANE_PASSWORD

  if (!username || !password) {
    throw new Error(
      'Missing LIVE_CONTROL_PLANE_USERNAME or LIVE_CONTROL_PLANE_PASSWORD.\n' +
      'These tests require REAL operator credentials.\n' +
      'See docs/LIVE_TESTING.md for setup instructions.'
    )
  }

  await page.goto('/admin/login', { waitUntil: 'networkidle' })

  // Wait for form to be ready and JavaScript to load
  await page.waitForSelector('#username', { state: 'visible' });
  await page.waitForTimeout(2000);

  // Fill the form
  const usernameInput = page.locator('#username');
  const passwordInput = page.locator('#password');
  
  await usernameInput.click();
  await usernameInput.fill(username);
  await passwordInput.click();
  await passwordInput.fill(password);
  
  // Submit by calling the login logic directly (the event listener doesn't work reliably in Playwright)
  await page.evaluate(async () => {
    const username = (document.getElementById('username') as HTMLInputElement).value;
    const password = (document.getElementById('password') as HTMLInputElement).value;
    
    // Get or create device ID
    let deviceId = '';
    try {
      const existing = localStorage.getItem('controlPlaneDeviceId');
      if (existing && existing.length >= 16) {
        deviceId = existing;
      } else {
        const bytes = new Uint8Array(32);
        if (window.crypto && window.crypto.getRandomValues) {
          window.crypto.getRandomValues(bytes);
        }
        const arr = Array.from(bytes);
        const bin = arr.map(b => String.fromCharCode(b)).join('');
        deviceId = btoa(bin).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/g, '');
        localStorage.setItem('controlPlaneDeviceId', deviceId);
      }
    } catch (e) {
      deviceId = 'test-device-id';
    }
    
    const deviceLabel = navigator.userAgent.slice(0, 120);
    
    const resp = await fetch('/api/v1/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password, deviceId, deviceLabel }),
    });
    
    if (!resp.ok) {
      throw new Error(`Login failed: ${resp.status}`);
    }
    
    const data = await resp.json();
    
    // Store the session token (control plane uses 'token', not 'accessToken')
    if (data.token) {
      localStorage.setItem('controlPlaneToken', data.token);
    }
    
    // Redirect to admin dashboard
    window.location.href = '/admin';
  });

  await page.waitForURL('**/admin', { timeout: 30_000 })
  await expect(page.locator('.brand', { hasText: 'APEX CONTROL PLANE' })).toBeVisible({ timeout: 10_000 })
  await expect(page.locator('#view-title')).toHaveText(/dashboard/i, { timeout: 10_000 })
}

test.describe('Control Plane LIVE — click everything (dummy operator)', () => {
  test.describe.configure({ timeout: 180_000 })

  test('login then click through operator dashboard UI', async ({ page, baseURL }) => {
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
      consoleErrors.push(text)
    })

    const serverErrors = await startServerErrorCapture(page, origin)

    await liveLogin(page)

    // Fail fast if login triggered a backend 5xx.
    expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)

    // On live we keep this to the operator UI surface and avoid any dangerous actions.
    await page.waitForLoadState('domcontentloaded')
    await clickVisibleButtons(page, { rounds: 3, maxClicks: 60, timeBudgetMs: 12_000 })

    await test.step('Sales Automation view renders', async () => {
      await page.locator('.nav-item[data-view="sales"]').click({ timeout: 5_000 })
      await expect(page.locator('#sales-contacts-count')).toBeVisible({ timeout: 10_000 })
    })

    await test.step('System Telemetry view stays logged in', async () => {
      await page.locator('.nav-item[data-view="telemetry"]').click({ timeout: 5_000 })
      await expect(page).toHaveURL(/\/admin(#telemetry)?$/)
      await expect(page.locator('.nav')).toBeVisible({ timeout: 5_000 })
      await expect(page.locator('#telemetry-content')).not.toHaveText(/Loading/)
      await expect(page.locator('#telemetry-content')).not.toHaveText(/Failed to load telemetry data/i)
      await expect(page.locator('#telemetry-content')).toContainText('"timestamp"')

      await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
      await expect(page).toHaveURL(/\/admin(#\w+)?$/)
      const token = await page.evaluate(() => localStorage.getItem('controlPlaneToken'))
      expect(token).toBeTruthy()
    })

    // Also exercise internal admin navigation links.
    const adminLinks = page.locator('a[href^="/admin"]')
    const linkCount = await adminLinks.count().catch(() => 0)
    for (let i = 0; i < linkCount; i++) {
      const link = adminLinks.nth(i)
      if (!(await link.isVisible().catch(() => false))) continue
      const href = (await link.getAttribute('href').catch(() => null))
      if (!href) continue
      if (href.includes('logout') || href.includes('signout')) continue

      await link.scrollIntoViewIfNeeded().catch(() => {})
      await link.click({ timeout: 750 }).catch(() => {})
      await page.waitForLoadState('domcontentloaded').catch(() => {})
      await clickVisibleButtons(page, { rounds: 2, maxClicks: 40, timeBudgetMs: 8_000 })
      await closeAnyModal(page)

      // Stop early with a useful error if any admin route triggers 5xx.
      expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0);
    }

    // Log any errors
    if (serverErrors.length > 0) {
      console.warn('Server errors:', serverErrors.length);
      serverErrors.forEach(err => console.log('  -', err));
    }
    if (pageErrors.length > 0) {
      console.warn('Page errors:', pageErrors);
    }
    if (consoleErrors.length > 0) {
      throw new Error(`Console errors detected:\n\n${consoleErrors.join('\n')}`)
    }
  })
})
