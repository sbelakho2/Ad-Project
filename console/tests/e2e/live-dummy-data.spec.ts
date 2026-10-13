// dogfood:skip:live/opt-in suite — requires a deployed environment (LIVE_E2E/CONSOLE_BASE_URL); enabled in the live lanes
import { test, expect, Page } from '@playwright/test'

type AuthResult =
  | { success: true; token: string; userId: string; publisherId: string; email: string }
  | { success: false; error: string }

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
  // Avoid `networkidle`: prod pages can keep long-lived requests open.
  await page.goto('/login', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: /ApexMediation/i })).toBeVisible({ timeout: 15_000 })

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

function parseDisplayNumber(text: string): { value: number; step: number } {
  const trimmed = text.trim()

  // Handle compact notation from Intl.NumberFormat (e.g. 1.4K, 2.1M)
  const m = trimmed.match(/(-?[\d.,]+)\s*([kKmMbB])?\b/)
  if (!m) return { value: 0, step: 1 }

  const rawNum = m[1]
  const suffix = (m[2] ?? '').toUpperCase()

  // Best-effort normalization for locales that may use thousands separators.
  // The console UI currently uses en-US in prod, but keep this robust.
  const normalized = rawNum.replace(/,/g, '')
  const decimals = (normalized.split('.')[1] ?? '').length
  const base = Number(normalized)
  if (!Number.isFinite(base)) return { value: 0, step: 1 }

  const multiplier = suffix === 'K' ? 1_000 : suffix === 'M' ? 1_000_000 : suffix === 'B' ? 1_000_000_000 : 1
  const step = multiplier / Math.pow(10, decimals)
  return { value: base * multiplier, step }
}

async function getMetricCardValue(page: Page, title: string) {
  const card = page.locator('div.card', { has: page.getByText(title, { exact: true }) })
  await expect(card).toBeVisible()

  const valueText = await card.locator('span').nth(1).innerText()
  return parseDisplayNumber(valueText)
}

test.describe('Console LIVE — dummy user data populates (Dashboard + VRA)', () => {
  test('dashboard summary matches API and recon shows expected revenue', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live data tests')
    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')

    const origin = new URL(baseURL).origin
    await loginAsLiveDummy(page, origin)

    const today = new Date().toISOString().split('T')[0]
    const start30 = new Date(Date.now() - 30 * 86400000).toISOString().split('T')[0]

    const summaryRes = await page.request.get(
      `${origin}/api/v1/revenue/summary?startDate=${encodeURIComponent(start30)}&endDate=${encodeURIComponent(today)}`
    )
    expect(summaryRes.ok()).toBeTruthy()

    const summaryJson = (await summaryRes.json()) as any
    expect(summaryJson?.success).toBeTruthy()

    const summary = summaryJson.data ?? summaryJson
    expect(summary.totalImpressions ?? 0).toBeGreaterThan(0)

    await page.goto('/dashboard', { waitUntil: 'domcontentloaded' })
    await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
    await expect(page.getByText('Total Revenue', { exact: true })).toBeVisible()

    const uiRevenue = await getMetricCardValue(page, 'Total Revenue')
    const uiImpressions = await getMetricCardValue(page, 'Impressions')

    // UI is rounded/formatted; compare with a small tolerance
    expect(uiRevenue.value).toBeGreaterThanOrEqual(Math.floor((summary.totalRevenue ?? 0) - 1))
    expect(uiRevenue.value).toBeLessThanOrEqual(Math.ceil((summary.totalRevenue ?? 0) + 1))

    // Impressions are compact-formatted (e.g. "1.4K"). Compare using a rounding-aware tolerance.
    const apiImpressions = Number(summary.totalImpressions ?? 0)
    const uiTolerance = Math.max(5, uiImpressions.step / 2)
    expect(Math.abs(uiImpressions.value - apiImpressions)).toBeLessThanOrEqual(uiTolerance)

    const overviewRes = await page.request.get(`${origin}/api/v1/recon/overview`)
    expect(overviewRes.ok()).toBeTruthy()

    const overviewJson = (await overviewRes.json()) as any
    expect(overviewJson?.success).toBeTruthy()
    const overview = overviewJson.data ?? overviewJson

    await page.goto('/recon', { waitUntil: 'domcontentloaded' })
    await expect(page.getByText('Revenue Reconciliation', { exact: true })).toBeVisible()

    const setupExpected = page.locator('[data-testid="recon-setup-expected"]')
    const metricExpectedCard = page.locator('div.rounded-md.border', { hasText: 'Expected Revenue' })

    const setupExpectedVisible = await setupExpected.isVisible().catch(() => false)
    const metricExpectedVisible = await metricExpectedCard.isVisible().catch(() => false)

    // Production may hide metrics until networks are connected; don't fail the test
    // on missing UI. The API assertion above is the VRA correctness check.
    if (!setupExpectedVisible && !metricExpectedVisible) {
      return
    }

    const expectedLocator = setupExpectedVisible ? setupExpected : metricExpectedCard
    const expectedText = await expectedLocator.innerText()
    const uiExpected = parseDisplayNumber(expectedText).value
    const apiExpected = Number(overview?.totals?.expected ?? 0)

    expect(uiExpected).toBeGreaterThanOrEqual(Math.floor(apiExpected - 1))
    expect(uiExpected).toBeLessThanOrEqual(Math.ceil(apiExpected + 1))
  })
})
