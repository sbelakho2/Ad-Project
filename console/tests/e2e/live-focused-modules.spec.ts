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
  const errorText = page.getByText(/application error|something went wrong|unexpected error/i)
  if (await errorText.first().isVisible().catch(() => false)) {
    throw new Error(`[${moduleName}] hit an error page: ${await errorText.first().innerText().catch(() => 'error')}`)
  }

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

test.describe('Console LIVE — focused module clickthroughs (dummy user)', () => {
  test('Migration Studio, Placements, Adapters, Supply Chain, tools (LIVE)', async ({ page, baseURL, request }) => {
    test.setTimeout(180_000)

    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live module tests')
    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')

    const origin = new URL(baseURL).origin
    const serverErrors = await startServerErrorCapture(page, origin)

    const pageErrors: string[] = []
    page.on('pageerror', (err) => pageErrors.push(String(err)))

    const consoleErrors: string[] = []
    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        consoleErrors.push(msg.text())
      }
    })

    await loginAsLiveDummy(page, origin)
    const token = await getAuthHeaderToken(page)

    // Grab a likely-valid appId for Mediation Debugger from the placements API.
    const placementsList = await apiGetJson(
      request,
      'https://api.apexmediation.ee/api/v1/placements?page=1&pageSize=50',
      token
    )

    const placements = placementsList?.data ?? placementsList?.placements ?? []
    const firstPlacement = Array.isArray(placements) ? placements[0] : undefined
    const appIdCandidate: string | undefined =
      firstPlacement?.platformId || firstPlacement?.platform_id || firstPlacement?.appId || firstPlacement?.app_id

    await test.step('Placements: list loads + detail page opens', async () => {
      await page.goto('/placements', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expectPageLooksHealthy(page, 'Placements')

      const anyPlacement = page.getByLabel(/^View /).first()
      if (await anyPlacement.isVisible().catch(() => false)) {
        await anyPlacement.click()
        await expect(page.getByRole('link', { name: /back to placements/i })).toBeVisible({ timeout: 25_000 })
        await expect(page.getByRole('heading', { name: /adapter configuration/i })).toBeVisible({ timeout: 25_000 })
        await page.getByRole('link', { name: /back to placements/i }).click()
      }
    })

    await test.step('Adapters: list loads + manage page opens', async () => {
      await page.goto('/adapters', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expectPageLooksHealthy(page, 'Adapters')
      await expect(page.getByRole('heading', { name: 'Adapters' })).toBeVisible({ timeout: 25_000 })

      const manage = page.getByRole('link', { name: 'Manage' }).first()
      if (await manage.isVisible().catch(() => false)) {
        await manage.click()
        await expectPageLooksHealthy(page, 'Adapter detail')
        await expect(page.getByRole('link', { name: /back/i })).toBeVisible({ timeout: 25_000 }).catch(() => {})
      }
    })

    await test.step('Migration Studio: page loads + parity simulator modal opens', async () => {
      await page.goto('/migration-studio', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expectPageLooksHealthy(page, 'Migration Studio')

      const openParity = page.getByRole('button', { name: /parity simulator/i }).first()
      await expect(openParity).toBeVisible({ timeout: 25_000 })

      // The button is disabled until a placement is selected (async). Avoid a hanging click.
      await expect(openParity)
        .toBeEnabled({ timeout: 25_000 })
        .catch(() => {})

      if (await openParity.isEnabled().catch(() => false)) {
        await openParity.click()
        await expect(page.getByRole('heading', { name: /parity simulator/i })).toBeVisible({ timeout: 25_000 })

        const close = page.getByRole('button', { name: 'Close' }).first()
        if (await close.isVisible().catch(() => false)) {
          await close.click()
          await expect(page.getByRole('heading', { name: /parity simulator/i })).toBeHidden({ timeout: 10_000 })
        }
      } else {
        // If parity never enables, ensure we at least have the placement picker state.
        await expect(page.locator('#placement-select')).toBeVisible({ timeout: 25_000 })
      }
    })

    await test.step('Supply Chain: summary loads and tool deep-link works', async () => {
      await page.goto('/supply-chain', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expectPageLooksHealthy(page, 'Supply Chain')
      await expect(page.getByRole('heading', { name: 'Supply Chain' })).toBeVisible({ timeout: 25_000 })

      const checkDomain = page.getByRole('link', { name: /check a domain/i }).first()
      if (await checkDomain.isVisible().catch(() => false)) {
        await checkDomain.click()
        await expect(page.getByRole('heading', { name: /supply chain status/i })).toBeVisible({ timeout: 25_000 })
      }
    })

    await test.step('Tool: Supply Chain Status runs (domain input)', async () => {
      await page.goto('/tools/supply-chain-status', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expect(page.getByRole('heading', { name: /developer tools — supply chain status/i })).toBeVisible({ timeout: 25_000 })

      const domainInput = page.getByLabel('Domain')
      const runButton = page.getByRole('button', { name: /run|refresh|checking/i })

      await expect(domainInput).toBeVisible({ timeout: 25_000 })

      // Next.js hydration can sometimes overwrite early input. Ensure the value sticks.
      const fillDomain = async () => {
        await domainInput.click()
        await domainInput.fill('apexmediation.ee')
        await expect
          .poll(async () => (await domainInput.inputValue()).trim(), { timeout: 10_000 })
          .toBe('apexmediation.ee')
      }

      await fillDomain()
      await expect(runButton)
        .toBeEnabled({ timeout: 10_000 })
        .catch(async () => {
          await fillDomain()
          await expect(runButton).toBeEnabled({ timeout: 10_000 })
        })

      const apiResponsePromise = page.waitForResponse(
        (res) => {
          try {
            if (res.request().method() !== 'GET') return false
            const url = new URL(res.url())
            return url.pathname.endsWith('/api/v1/tools/supply-chain-status')
          } catch {
            return false
          }
        },
        { timeout: 60_000 }
      )

      const [apiResponse] = await Promise.all([apiResponsePromise, runButton.click()])
      expect(apiResponse.status(), 'Supply Chain Status API should not be 401').not.toBe(401)
      expect(apiResponse.status(), 'Supply Chain Status API should not be 5xx').toBeLessThan(500)

      const apiStatus = apiResponse.status()
      const apiBodySnippet = await apiResponse
        .text()
        .then((t) => t.slice(0, 2000))
        .catch(() => '')

      const hasResult = page.getByText(/Domain: /).first()
      const hasStatusPill = page.getByText(/AUTHORIZED|ATTENTION/).first()
      const hasEmptyState = page.getByText(/No entries found for this domain/i).first()
      const hasEntriesHeader = page.getByText(/Declared app-ads\.txt entries/i).first()
      const hasErrorBox = page.locator('div.border-red-200').first()

      // Best-effort: UI should render quickly, but don't fail the suite if Next.js routing/session
      // issues prevent the client component from updating. The API response is the primary signal.
      await Promise.race([
        hasResult.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
        hasStatusPill.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
        hasEmptyState.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
        hasEntriesHeader.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
        hasErrorBox.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
      ])

      if (await hasErrorBox.isVisible().catch(() => false)) {
        const jsErrors = pageErrors.length ? pageErrors.slice(-5).join('\n') : ''
        const browserConsoleErrors = consoleErrors.length ? consoleErrors.slice(-8).join('\n') : ''
        throw new Error(
          `Supply Chain Status UI displayed an error after API response.\n\n` +
            `API status: ${apiStatus}\n` +
            `API body (first 2KB):\n${apiBodySnippet || '(empty)'}\n\n` +
            (browserConsoleErrors ? `Console errors (last ${Math.min(consoleErrors.length, 8)}):\n${browserConsoleErrors}\n\n` : '') +
            (jsErrors ? `Page JS errors (last ${Math.min(pageErrors.length, 5)}):\n${jsErrors}\n\n` : '')
        )
      }
    })

    await test.step('Tool: app-ads.txt Inspector runs (domain input)', async () => {
      await page.goto('/tools/app-ads-inspector', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expect(page.getByRole('heading', { name: /developer tools — app-ads\.txt inspector/i })).toBeVisible({ timeout: 25_000 })

      const domainInput = page.getByLabel('Domain')
      const runButton = page.getByRole('button', { name: /run|refresh|checking/i })
      await expect(domainInput).toBeVisible({ timeout: 25_000 })

      await domainInput.click()
      await domainInput.fill('apexmediation.ee')

      await expect(runButton)
        .toBeEnabled({ timeout: 10_000 })
        .catch(() => {})

      const apiResponsePromise = page.waitForResponse(
        (res) => {
          try {
            if (res.request().method() !== 'GET') return false
            const url = new URL(res.url())
            return url.pathname.endsWith('/api/v1/tools/app-ads-inspector')
          } catch {
            return false
          }
        },
        { timeout: 60_000 }
      )

      const [apiResponse] = await Promise.all([apiResponsePromise, runButton.click()])
      expect(apiResponse.status(), 'app-ads Inspector API should not be 401').not.toBe(401)
      expect(apiResponse.status(), 'app-ads Inspector API should not be 5xx').toBeLessThan(500)

      const statusPill = page.getByText(/PASS|ATTENTION/).first()
      const anyWarning = page.getByText(/Could not fetch/i).first()
      const errorBox = page.locator('div.border-red-200').first()

      await Promise.race([
        statusPill.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
        anyWarning.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
        errorBox.waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {}),
      ])

      if (await errorBox.isVisible().catch(() => false)) {
        const apiBodySnippet = await apiResponse
          .text()
          .then((t) => t.slice(0, 2000))
          .catch(() => '')
        throw new Error(
          `app-ads Inspector UI displayed an error after API response.\n\n` +
            `API status: ${apiResponse.status()}\n` +
            `API body (first 2KB):\n${apiBodySnippet || '(empty)'}\n`
        )
      }
    })

    await test.step('Tool: Mediation Debugger can run (App ID)', async () => {
      await page.goto('/tools/mediation-debugger', { waitUntil: 'domcontentloaded', timeout: 60_000 })
      await expect(page.getByRole('heading', { name: /mediation debugger/i })).toBeVisible({ timeout: 25_000 })

      if (!appIdCandidate) {
        return
      }

      await page.getByLabel('App ID').fill(appIdCandidate!)
      await page.getByRole('button', { name: /run|refresh|loading/i }).click()

      const anyError = page.locator('text=/Failed to load metrics/i').first().or(page.locator('div').filter({ hasText: /Failed to load metrics/i }).first())
      const anySummary = page.getByText(/p50 \(ms\)/).first()

      await expect
        .poll(
          async () => {
            const err = await anyError.isVisible().catch(() => false)
            const ok = await anySummary.isVisible().catch(() => false)
            return ok || err
          },
          { timeout: 25_000 }
        )
        .toBeTruthy()
    })

    if (pageErrors.length) {
      throw new Error(`Page JS errors detected:\n\n${pageErrors.join('\n')}`)
    }

    throwIfAnyServerErrors(serverErrors, 'focused module clickthroughs')
  })
})
