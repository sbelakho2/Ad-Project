// dogfood:skip:live/opt-in suite — requires a deployed environment (LIVE_E2E/CONSOLE_BASE_URL); enabled in the live lanes
import { test, expect, Page, APIRequestContext } from '@playwright/test'

type AuthResult =
  | { success: true; token: string; userId: string; publisherId: string; email: string }
  | { success: false; error: string }

async function startServerErrorCapture(page: Page, origin: string) {
  const serverErrors: string[] = []

  page.on('response', async (res) => {
    try {
      const status = res.status()
      const url = res.url()
      if (!url.startsWith(origin)) return

      // Hard failures: 5xx from our origin.
      if (status >= 500) {
        let snippet = ''
        try {
          snippet = (await res.text()).slice(0, 2000)
        } catch {
          // ignore
        }
        serverErrors.push(`${status} ${url}${snippet ? `\n${snippet}` : ''}`)
        return
      }

      // Auth failures should never happen after live login.
      // 403 can be a legitimate plan gate; 401 generally means broken auth.
      if (status === 401) {
        serverErrors.push(`${status} ${url}`)
      }
    } catch {
      // best-effort
    }
  })

  return serverErrors
}

function throwIfAnyServerErrors(serverErrors: string[], context: string) {
  if (serverErrors.length === 0) return
  throw new Error(`Server errors detected during ${context}:\n\n${serverErrors.join('\n\n')}`)
}

async function loginAsLiveDummy(page: Page, origin: string) {
  const email = process.env.LIVE_CONSOLE_EMAIL
  const password = process.env.LIVE_CONSOLE_PASSWORD

  if (!email || !password) {
    throw new Error(
      'Missing LIVE_CONSOLE_EMAIL or LIVE_CONSOLE_PASSWORD.\n' +
        'See docs/LIVE_TESTING.md for setup instructions.'
    )
  }

  // Ensure we have a real document origin before doing cross-origin XHR.
  await page.goto('/login', { waitUntil: 'networkidle' })

  const authResult = await page.evaluate<AuthResult, { email: string; password: string }>(
    async ({ email, password }) => {
      return new Promise((resolve) => {
        try {
          const xhr = new XMLHttpRequest()
          xhr.open('POST', 'https://api.apexmediation.ee/api/v1/auth/login', true)
          xhr.setRequestHeader('Content-Type', 'application/json')
          xhr.withCredentials = true

          xhr.onload = function () {
            try {
              if (xhr.status !== 200) {
                resolve({ success: false, error: `Login failed: ${xhr.status} ${xhr.responseText}` })
                return
              }

              const data = JSON.parse(xhr.responseText)
              const token = data.data?.token || data.token

              if (!token) {
                resolve({ success: false, error: 'No token in response' })
                return
              }

              const xhr2 = new XMLHttpRequest()
              xhr2.open('GET', 'https://api.apexmediation.ee/api/v1/auth/me', true)
              xhr2.setRequestHeader('Authorization', `Bearer ${token}`)
              xhr2.withCredentials = true

              xhr2.onload = function () {
                try {
                  if (xhr2.status !== 200) {
                    resolve({ success: false, error: `Failed to get user info: ${xhr2.status}` })
                    return
                  }

                  const userData = JSON.parse(xhr2.responseText)
                  const user = userData.data || userData

                  localStorage.setItem('auth_token', token)
                  localStorage.setItem('access_token', token)
                  localStorage.setItem('user_id', user.userId)
                  localStorage.setItem('publisher_id', user.publisherId)
                  localStorage.setItem('user_email', user.email)

                  resolve({
                    success: true,
                    token,
                    userId: user.userId,
                    publisherId: user.publisherId,
                    email: user.email,
                  })
                } catch (err) {
                  resolve({ success: false, error: `User info parse error: ${err}` })
                }
              }

              xhr2.onerror = function () {
                resolve({ success: false, error: 'User info request failed' })
              }

              xhr2.send()
            } catch (err) {
              resolve({ success: false, error: `Login parse error: ${err}` })
            }
          }

          xhr.onerror = function () {
            resolve({ success: false, error: 'Login request failed' })
          }

          xhr.send(JSON.stringify({ email, password }))
        } catch (error) {
          resolve({ success: false, error: String(error) })
        }
      })
    },
    { email, password }
  )

  if (!authResult.success) {
    throw new Error(`Authentication failed: ${authResult.error}`)
  }

  const domain = new URL(origin).hostname
  const isSecure = origin.startsWith('https')

  await page.context().addCookies([
    {
      name: 'access_token',
      value: authResult.token,
      domain,
      path: '/',
      httpOnly: true,
      secure: isSecure,
      sameSite: 'Lax',
    },
  ])

  return authResult
}

async function expectPageLooksHealthy(page: Page, moduleName: string) {
  // Avoid passing when the app is on a generic error page.
  const errorText = page.getByText(/application error|something went wrong|unexpected error/i)
  if (await errorText.first().isVisible().catch(() => false)) {
    throw new Error(`[${moduleName}] hit an error page: ${await errorText.first().innerText().catch(() => 'error')}`)
  }

  // Module is considered "working" if it shows either module content
  // (H1/alert/main) OR the authenticated app shell (sidebar/sign out).
  const h1 = page.locator('h1').first()
  const alert = page.locator('[role="alert"]').first()
  const main = page.locator('main').first()
  const mainNav = page.getByRole('navigation', { name: /main navigation/i }).first()
  const signOut = page.getByRole('button', { name: /sign out/i }).first()

  await expect
    .poll(
      async () => {
        const hasH1 = await h1.isVisible().catch(() => false)
        const hasAlert = await alert.isVisible().catch(() => false)
        const hasMain = await main.isVisible().catch(() => false)
        const hasNav = await mainNav.isVisible().catch(() => false)
        const hasSignOut = await signOut.isVisible().catch(() => false)
        return hasH1 || hasAlert || hasMain || hasNav || hasSignOut
      },
      { timeout: 25_000 }
    )
    .toBeTruthy()
}

function parseMoney(text: string): number {
  const cleaned = text.replace(/[^0-9.,-]/g, '').replace(/,/g, '')
  const v = Number.parseFloat(cleaned)
  return Number.isFinite(v) ? v : 0
}

async function getAuthHeaderToken(page: Page): Promise<string> {
  const token = await page.evaluate(() => {
    return localStorage.getItem('access_token') || localStorage.getItem('auth_token') || ''
  })
  if (!token) throw new Error('Missing auth token in localStorage after login')
  return token
}

async function apiGetJson(request: APIRequestContext, url: string, token: string) {
  const res = await request.get(url, {
    headers: { Authorization: `Bearer ${token}` },
  })
  expect(res.ok(), `GET ${url} failed: ${res.status()}`).toBeTruthy()
  return res.json() as Promise<any>
}

test.describe('Console LIVE — modules + VRA end-to-end (dummy user)', () => {
  test('visit each module and validate VRA uses live data', async ({ page, baseURL, request }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live module tests')
    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')

    const origin = new URL(baseURL).origin
    const serverErrors = await startServerErrorCapture(page, origin)

    const pageErrors: string[] = []
    page.on('pageerror', (err) => {
      pageErrors.push(String(err))
    })

    await loginAsLiveDummy(page, origin)
    const token = await getAuthHeaderToken(page)

    // Modules derived from Navigation + Settings tree + VRA routes.
    const modules: Array<{ name: string; path: string }> = [
      { name: 'Dashboard', path: '/dashboard' },
      { name: 'Placements', path: '/placements' },
      { name: 'Migration Studio', path: '/migration-studio' },
      { name: 'Adapters', path: '/adapters' },
      { name: 'Analytics', path: '/analytics' },
      { name: 'Transparency', path: '/transparency/auctions' },
      { name: 'Fraud Detection', path: '/fraud' },
      { name: 'Supply Chain', path: '/supply-chain' },
      { name: 'Tool: Supply Chain Status', path: '/tools/supply-chain-status' },
      { name: 'Tool: app-ads.txt Inspector', path: '/tools/app-ads-inspector' },
      { name: 'Tool: Mediation Debugger', path: '/tools/mediation-debugger' },
      { name: 'Settings', path: '/settings' },
      { name: 'Settings: Networks', path: '/settings/networks' },
      { name: 'Settings: Billing', path: '/settings/billing/settings' },
      { name: 'Settings: API Keys', path: '/settings/keys' },
      { name: 'Settings: Team', path: '/settings/team' },
      { name: 'Settings: Notifications', path: '/settings/notifications' },
      { name: 'Settings: SDK', path: '/settings/sdk' },
      { name: 'Settings: Security', path: '/settings/security' },
      { name: 'Settings: Compliance', path: '/settings/compliance' },
      { name: 'Settings: Credentials', path: '/settings/credentials' },
      { name: 'Settings: Fraud', path: '/settings/fraud' },

      // VRA / reconciliation
      { name: 'VRA: Recon Dashboard', path: '/recon' },
      { name: 'VRA: Overview', path: '/recon/overview' },
      { name: 'VRA: Deltas', path: '/recon/deltas' },
    ]

    for (const mod of modules) {
      await test.step(mod.name, async () => {
        await page.goto(mod.path, { waitUntil: 'domcontentloaded', timeout: 60_000 })
        await expect(page, `[${mod.name}] should not end on login`).not.toHaveURL(/\/login(\?|$)/)
        await expectPageLooksHealthy(page, mod.name)
      })
    }

    // VRA end-to-end: compare API totals to UI totals on /recon/overview.
    await test.step('VRA totals: API vs UI', async () => {
      await page.goto('/recon/overview', { waitUntil: 'domcontentloaded' })
      await expect(page.getByRole('heading', { name: /reconciliation overview/i })).toBeVisible({ timeout: 20_000 })

      const upgradeAlert = page
        .locator('[role="alert"]')
        .filter({ hasText: /upgrade to access vra reconciliation/i })
        .first()

      const totalsSection = page.locator('section', { has: page.getByText('Totals', { exact: true }) }).first()

      // On live, billing/plan access is often determined asynchronously.
      // Wait briefly for either the plan gate OR the totals section to appear.
      await expect
        .poll(
          async () => {
            const gated = await upgradeAlert.isVisible().catch(() => false)
            const hasTotals = await totalsSection.isVisible().catch(() => false)
            return gated || hasTotals
          },
          { timeout: 15_000 }
        )
        .toBeTruthy()

      // If the dummy user is on Starter, the module is gated by design.
      // In that case, validate the gate renders and skip the data comparison.
      if (await upgradeAlert.isVisible().catch(() => false)) {
        await expect(upgradeAlert).toBeVisible()
        return
      }

      const api = await apiGetJson(request, `${origin}/api/v1/recon/overview`, token)
      expect(api?.success).toBeTruthy()
      const overview = api.data ?? api

      // Totals section
      const paidText = await totalsSection
        .getByText(/paid/i)
        .locator('..')
        .innerText()
        .catch(async () => totalsSection.innerText())
      const expectedText = await totalsSection
        .getByText(/expected/i)
        .locator('..')
        .innerText()
        .catch(async () => totalsSection.innerText())

      const uiPaid = parseMoney(paidText)
      const uiExpected = parseMoney(expectedText)

      const apiPaid = Number(overview?.totals?.paid ?? 0)
      const apiExpected = Number(overview?.totals?.expected ?? 0)

      // UI rounds/formatting; allow $1 drift.
      expect(Math.abs(uiPaid - apiPaid)).toBeLessThanOrEqual(1)
      expect(Math.abs(uiExpected - apiExpected)).toBeLessThanOrEqual(1)
    })

    await test.step('VRA deltas CSV is reachable', async () => {
      // If plan-gated, UI will show upgrade alert and API may 403. Treat that as "working as intended".
      const res = await request.get(`${origin}/api/v1/recon/deltas.csv`, {
        headers: { Authorization: `Bearer ${token}` },
      })

      if (res.status() === 403) {
        return
      }

      expect(res.ok(), `deltas.csv status=${res.status()}`).toBeTruthy()
      const body = await res.text()
      expect(body.length).toBeGreaterThan(0)
    })

    throwIfAnyServerErrors(serverErrors, 'module traversal')

    // Fail on uncaught browser exceptions.
    expect(pageErrors, pageErrors.join('\n')).toHaveLength(0)
  })
})
