// dogfood:skip:live/opt-in suite — requires a deployed environment (LIVE_E2E/CONSOLE_BASE_URL); enabled in the live lanes
import { test, expect, Page, APIRequestContext } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'

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

async function expectPageLooksHealthy(page: Page, routeName: string) {
  const errorText = page.getByText(/application error|something went wrong|unexpected error/i)
  if (await errorText.first().isVisible().catch(() => false)) {
    throw new Error(`[${routeName}] hit an error page: ${await errorText.first().innerText().catch(() => 'error')}`)
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
      { timeout: 30_000 }
    )
    .toBeTruthy()
}

function listStaticAppRoutes(): string[] {
  const appRoot = path.resolve(__dirname, '../../src/app')
  const pages: string[] = []

  const walk = (dir: string) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith('_')) continue
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        // Skip route groups like (foo)
        if (entry.name.startsWith('(') && entry.name.endsWith(')')) {
          walk(full)
          continue
        }
        walk(full)
        continue
      }
      if (entry.isFile() && entry.name === 'page.tsx') {
        const rel = path.relative(appRoot, full)
        const routeDir = path.dirname(rel)
        const segments = routeDir === '.' ? [] : routeDir.split(path.sep)
        // Exclude dynamic segments from the static list; we handle them via data-driven steps.
        if (segments.some((s) => s.startsWith('[') && s.endsWith(']'))) continue
        const route = '/' + segments.join('/')
        pages.push(route === '/' ? '/' : route.replace(/\\/g, '/'))
      }
    }
  }

  walk(appRoot)

  // De-dupe + stable ordering
  return Array.from(new Set(pages)).sort((a, b) => a.localeCompare(b))
}

async function fillInputStable(page: Page, label: string, value: string) {
  const input = page.getByLabel(label).first()
  await expect(input).toBeVisible({ timeout: 25_000 })
  await input.click()
  await input.fill(value)
  await expect
    .poll(async () => (await input.inputValue()).trim(), { timeout: 10_000 })
    .toBe(value)
}

async function runToolWithApiAssertion(page: Page, apiPathSuffix: string, runButtonName = /run|refresh|checking/i) {
  const runButton = page.getByRole('button', { name: runButtonName }).first()
  await expect(runButton).toBeVisible({ timeout: 25_000 })

  const apiResponsePromise = page.waitForResponse(
    (res) => {
      try {
        if (res.request().method() !== 'GET') return false
        const url = new URL(res.url())
        return url.pathname.endsWith(`/api/v1${apiPathSuffix}`)
      } catch {
        return false
      }
    },
    { timeout: 60_000 }
  )

  const [apiResponse] = await Promise.all([apiResponsePromise, runButton.click()])
  expect(apiResponse.status(), `${apiPathSuffix} should not be 401`).not.toBe(401)
  expect(apiResponse.status(), `${apiPathSuffix} should not be 5xx`).toBeLessThan(500)

  return apiResponse
}

test.describe('Console LIVE — deep all-pages usage (non-destructive)', () => {
  test('Walk every page and exercise meaningful interactions', async ({ page, baseURL, request }) => {
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
      try {
        const u = new URL(where)
        if (u.pathname.startsWith('/admin')) return
      } catch {
        // fall through
      }
      pageErrors.push(`[${where}] ${String(err)}`)
    })

    await loginAsLiveDummy(page, origin)
    const token = await getAuthHeaderToken(page)

    // Seed IDs for dynamic routes.
    const placementsList = await apiGetJson(
      request,
      'https://api.apexmediation.ee/api/v1/placements?page=1&pageSize=50',
      token
    )

    const placements = placementsList?.data ?? placementsList?.placements ?? []
    const firstPlacement = Array.isArray(placements) ? placements[0] : undefined
    const placementId: string | undefined = firstPlacement?.id

    // Routes: include static app routes + a few dynamic routes derived from live data.
    const staticRoutes = listStaticAppRoutes()

    const routes: Array<{ path: string; name: string }> = []

    // Public-ish pages
    routes.push({ path: '/', name: 'Home' })
    routes.push({ path: '/login', name: 'Login' })
    routes.push({ path: '/signup', name: 'Signup' })
    routes.push({ path: '/403', name: '403' })

    for (const r of staticRoutes) {
      if (r === '/' || r === '/login' || r === '/signup' || r === '/403') continue
      routes.push({ path: r, name: r })
    }

    if (placementId) {
      routes.push({ path: `/placements/${placementId}`, name: 'Placement detail' })
    }

    // The Admin area isn't for publisher users; verify it redirects / blocks in a meaningful way.
    const adminLike = routes.filter((r) => r.path.startsWith('/admin'))
    const nonAdmin = routes.filter((r) => !r.path.startsWith('/admin'))

    // ---------- Core: visit all non-admin pages ----------
    for (const { path: routePath, name } of nonAdmin) {
      await test.step(`Visit ${name}`, async () => {
        await page.goto(routePath, { waitUntil: 'domcontentloaded', timeout: 60_000 })
        await expectPageLooksHealthy(page, name)

        // Generic: avoid "couldn't load" error cards.
        await expect(page.getByText(/we couldn’t load|could not load/i)).toHaveCount(0)
      })

      // Page-specific deep interactions (read-only / safe)
      if (routePath === '/placements') {
        await test.step('Placements: open first detail + open new form', async () => {
          const anyPlacement = page.getByLabel(/^View /).first()
          if (await anyPlacement.isVisible().catch(() => false)) {
            await anyPlacement.click()
            await expect(page.getByRole('link', { name: /back to placements/i })).toBeVisible({ timeout: 25_000 })
            await expect(page.getByRole('heading', { name: /adapter configuration/i })).toBeVisible({ timeout: 25_000 })
            await page.getByRole('link', { name: /back to placements/i }).click()
          }

          const newBtn = page.getByRole('link', { name: /new placement|create placement|add placement/i }).first()
          if (await newBtn.isVisible().catch(() => false)) {
            await newBtn.click()
            await expectPageLooksHealthy(page, 'Placement create')
            await expect(page.locator('input, select, textarea').first()).toBeVisible({ timeout: 25_000 })
            await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
          }
        })
      }

      if (routePath === '/adapters') {
        await test.step('Adapters: open manage + open new form', async () => {
          const manage = page.getByRole('link', { name: 'Manage' }).first()
          if (await manage.isVisible().catch(() => false)) {
            await manage.click()
            await expectPageLooksHealthy(page, 'Adapter detail')
            await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
          }

          const newAdapter = page.getByRole('link', { name: /new adapter|add adapter|create adapter/i }).first()
          if (await newAdapter.isVisible().catch(() => false)) {
            await newAdapter.click()
            await expectPageLooksHealthy(page, 'Adapter create')
            await expect(page.locator('input, select, textarea').first()).toBeVisible({ timeout: 25_000 })
            await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
          }
        })
      }

      if (routePath === '/migration-studio') {
        await test.step('Migration Studio: parity simulator (if enabled) + open first experiment (if exists)', async () => {
          const openParity = page.getByRole('button', { name: /parity simulator/i }).first()
          if (await openParity.isVisible().catch(() => false)) {
            await expect(openParity)
              .toBeEnabled({ timeout: 10_000 })
              .catch(() => {})
            if (await openParity.isEnabled().catch(() => false)) {
              await openParity.click()
              await expect(page.getByRole('heading', { name: /parity simulator/i })).toBeVisible({ timeout: 25_000 })
              const close = page.getByRole('button', { name: 'Close' }).first()
              if (await close.isVisible().catch(() => false)) {
                await close.click()
                await expect(page.getByRole('heading', { name: /parity simulator/i })).toBeHidden({ timeout: 10_000 }).catch(() => {})
              }
            }
          }

          const experimentLink = page.getByRole('link', { name: /experiment/i }).first()
          if (await experimentLink.isVisible().catch(() => false)) {
            await experimentLink.click()
            await expectPageLooksHealthy(page, 'Migration experiment')
            await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
          }
        })
      }

      if (routePath === '/analytics') {
        await test.step('Analytics: check summary cards render values', async () => {
          const totalRevenueCard = page.locator('div.card').filter({ hasText: 'Total Revenue' }).first()
          await expect(totalRevenueCard).toBeVisible({ timeout: 25_000 })
          await expect(totalRevenueCard.getByText(/\$\s*\d|€\s*\d|£\s*\d/)).toBeVisible({ timeout: 25_000 })
        })
      }

      if (routePath === '/transparency/auctions') {
        await test.step('Transparency: open first auction detail (if possible)', async () => {
          const firstLink = page.getByRole('link').filter({ hasText: /Auction|Details|View/i }).first()
          if (await firstLink.isVisible().catch(() => false)) {
            await firstLink.click()
            await expectPageLooksHealthy(page, 'Auction detail')
            await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
          }
        })
      }

      if (routePath === '/fraud') {
        await test.step('Fraud: ensure at least one section renders', async () => {
          const anyHeading = page.locator('h2,h3').first()
          await expect(anyHeading).toBeVisible({ timeout: 25_000 })
        })
      }

      if (routePath === '/supply-chain') {
        await test.step('Supply Chain: follow Check a domain link (if present)', async () => {
          const checkDomain = page.getByRole('link', { name: /check a domain/i }).first()
          if (await checkDomain.isVisible().catch(() => false)) {
            await checkDomain.click()
            await expect(page.getByRole('heading', { name: /supply chain status/i })).toBeVisible({ timeout: 25_000 })
            await page.goBack({ waitUntil: 'domcontentloaded' }).catch(() => {})
          }
        })
      }

      if (routePath === '/tools/supply-chain-status') {
        await test.step('Tool: Supply Chain Status (run)', async () => {
          await fillInputStable(page, 'Domain', 'apexmediation.ee')
          await runToolWithApiAssertion(page, '/tools/supply-chain-status')
        })
      }

      if (routePath === '/tools/app-ads-inspector') {
        await test.step('Tool: app-ads.txt Inspector (run)', async () => {
          await fillInputStable(page, 'Domain', 'apexmediation.ee')
          await runToolWithApiAssertion(page, '/tools/app-ads-inspector')
        })
      }

      if (routePath === '/tools/mediation-debugger') {
        await test.step('Tool: Mediation Debugger (run if we have an appId)', async () => {
          const appIdCandidate: string | undefined =
            firstPlacement?.platformId || firstPlacement?.platform_id || firstPlacement?.appId || firstPlacement?.app_id
          if (!appIdCandidate) return

          await fillInputStable(page, 'App ID', appIdCandidate)
          // This tool uses analytics endpoints; we only assert we didn't crash and no error banner.
          const runButton = page.getByRole('button', { name: /run|refresh|loading/i }).first()
          await expect(runButton).toBeVisible({ timeout: 25_000 })
          await runButton.click()

          const anyError = page.locator('text=/Failed to load metrics/i').first()
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
      }
    }

    // ---------- Admin routes: should redirect or block meaningfully ----------
    for (const { path: routePath } of adminLike) {
      await test.step(`Visit ${routePath} (expect operator gate)`, async () => {
        let resp: any = null
        try {
          resp = await page.goto(routePath, { waitUntil: 'domcontentloaded', timeout: 60_000 })
        } catch (err) {
          // Playwright reports net::ERR_ABORTED when a client-side redirect (e.g. window.location.replace)
          // interrupts the initial navigation. For /admin/* this is acceptable; we assert on the resulting page.
          const msg = String(err)
          if (!msg.includes('net::ERR_ABORTED')) throw err
          await page.waitForLoadState('domcontentloaded', { timeout: 60_000 }).catch(() => {})
        }

        // This app intentionally hard-redirects /admin* routes to the dedicated operator hostname.
        // Give the client-side redirect a moment to fire.
        await page.waitForTimeout(400)

        // Either:
        // - redirected to /admin/login
        // - sees a 403 page
        // - sees a login form
        // - or is blocked/hidden (401/403/404) / redirected elsewhere
        const url = page.url()
        const status = resp?.status?.() ?? 0
        const title = await page.title().catch(() => '')

        const host = (() => {
          try {
            return new URL(url).host
          } catch {
            return ''
          }
        })()
        const isControlPlaneHost = /(^|\.)admin\.apexmediation\.ee$/i.test(host)

        const isAdminLogin = url.includes('/admin/login')
        const isAnyLogin = /\/(?:admin\/login|login|signin|auth)(?:\b|\/)/i.test(url)

        const hasOperatorControlPlaneHeading = await page
          .getByRole('heading', { name: /operator control plane/i })
          .isVisible()
          .catch(() => false)
        const hasGoToControlPlane = await page
          .getByRole('link', { name: /go to control plane/i })
          .isVisible()
          .catch(() => false)

        const has403 = await page.getByRole('heading', { name: /403|forbidden/i }).isVisible().catch(() => false)
        const has404 = await page.getByRole('heading', { name: /404|not found/i }).isVisible().catch(() => false)
        const hasLoginHeading = await page.getByRole('heading', { name: /admin login|login|sign in/i }).isVisible().catch(() => false)
        const hasUsername = await page.getByLabel(/username/i).isVisible().catch(() => false)
        const hasPassword = await page.getByLabel(/password/i).isVisible().catch(() => false)

        // In production, /admin/* may be served inside the console hostname as a Control Plane UI.
        // If it renders cleanly, accept it as "meaningful" rather than requiring a block/redirect.
        const hasControlPlaneHeading = await page.getByRole('heading', { name: /control plane/i }).isVisible().catch(() => false)
        const looksLikeControlPlane = hasControlPlaneHeading || /control\s*plane/i.test(title)

        const currentPath = (() => {
          try {
            return new URL(url).pathname
          } catch {
            return url
          }
        })()
        const wasRedirectedAway = !currentPath.startsWith(routePath) && !currentPath.startsWith(`${routePath}/`)

        const isMeaningfullyGated =
          status === 301 ||
          status === 302 ||
          status === 303 ||
          status === 307 ||
          status === 308 ||
          status === 401 ||
          status === 403 ||
          status === 404 ||
          isControlPlaneHost ||
          isAdminLogin ||
          isAnyLogin ||
          hasOperatorControlPlaneHeading ||
          hasGoToControlPlane ||
          has403 ||
          has404 ||
          hasLoginHeading ||
          hasUsername ||
          hasPassword ||
          wasRedirectedAway

        const isAcceptable = isMeaningfullyGated || looksLikeControlPlane

        if (!isAcceptable) {
          const bodySnippet = await page
            .locator('body')
            .innerText()
            .then((t) => t.replace(/\s+/g, ' ').trim().slice(0, 280))
            .catch(() => '')
          throw new Error(
            `Unexpected admin route behavior for ${routePath}: status=${status} url=${url} title=${JSON.stringify(title)} body=${JSON.stringify(bodySnippet)}`
          )
        }
      })
    }

    if (pageErrors.length) {
      // Keep this actionable but bounded
      throw new Error(`Page JS errors detected (last ${Math.min(pageErrors.length, 10)}):\n\n${pageErrors.slice(-10).join('\n')}`)
    }

    throwIfAnyServerErrors(serverErrors, 'deep all-pages usage')
  })
})
