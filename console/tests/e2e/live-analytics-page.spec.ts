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

  await page.goto('/login', { waitUntil: 'domcontentloaded' })

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

test.describe('Console LIVE — Analytics page', () => {
  test('Analytics loads and shows real metrics', async ({ page, baseURL }) => {
    test.setTimeout(120_000)

    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live tests')
    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')

    const origin = new URL(baseURL).origin
    await loginAsLiveDummy(page, origin)

    await page.goto('/analytics', { waitUntil: 'domcontentloaded', timeout: 60_000 })

    await expect(page.getByRole('heading', { name: 'Analytics' })).toBeVisible({ timeout: 25_000 })
    await expect(page.getByText('Total Revenue')).toBeVisible({ timeout: 25_000 })
    await expect(page.getByRole('heading', { name: 'Revenue Trend' })).toBeVisible({ timeout: 25_000 })

    // Avoid false positives: page should not be in an error state.
    await expect(page.getByText(/we couldn’t load/i)).toHaveCount(0)

    // “Meaningful output”: Total Revenue value should render as a currency.
    const totalRevenueCard = page
      .locator('div.card')
      .filter({ hasText: 'Total Revenue' })
      .first()

    await expect(totalRevenueCard).toBeVisible({ timeout: 25_000 })
    await expect(totalRevenueCard.getByText(/\$\s*\d|€\s*\d|£\s*\d/)).toBeVisible({ timeout: 25_000 })

    // Chart should either render data figure or explicit "No data" empty state.
    const chartFigure = page.getByRole('figure', { name: /revenue trend/i })
    const emptyState = page.getByText(/no data available for the selected period/i).first()

    await expect
      .poll(
        async () => {
          const hasFigure = await chartFigure.isVisible().catch(() => false)
          const hasEmpty = await emptyState.isVisible().catch(() => false)
          return hasFigure || hasEmpty
        },
        { timeout: 25_000 }
      )
      .toBeTruthy()
  })
})
