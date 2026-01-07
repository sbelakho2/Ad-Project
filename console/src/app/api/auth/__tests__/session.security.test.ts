import type { NextAuthOptions } from 'next-auth'
import { AxiosError } from 'axios'
import { apiClient, AUTH_UNAUTHORIZED_EVENT } from '@/lib/api-client'
import { getCsrfToken, readXsrfCookie } from '@/lib/csrf'

jest.mock('next-auth/react', () => ({
  __esModule: true,
  getSession: jest.fn(async () => null),
}))

jest.mock('next-auth', () => ({
  __esModule: true,
  default: jest.fn(() => ({})),
}))

jest.mock('next-auth/providers/credentials', () => ({
  __esModule: true,
  default: jest.fn((config) => ({
    id: 'credentials',
    name: 'Credentials',
    type: 'credentials',
    authorize: config.authorize,
    credentials: config.credentials,
  })),
}))

jest.mock('@/lib/csrf', () => ({
  __esModule: true,
  getCsrfToken: jest.fn(),
  readXsrfCookie: jest.fn(),
  XSRF_COOKIE_NAME: 'XSRF-TOKEN',
}))

const mockedGetCsrfToken = jest.mocked(getCsrfToken)
const mockedReadXsrfCookie = jest.mocked(readXsrfCookie)

type EnvOverrides = Partial<Record<'NEXT_PUBLIC_USE_MOCK_API' | 'NEXT_PUBLIC_ALLOW_DEMO_AUTH', string>>
type LoadAuthOptionsParams = {
  nodeEnv?: string
  env?: EnvOverrides
  apiClientMock?: { post: jest.Mock }
}

const ENV_KEYS: Array<keyof NodeJS.ProcessEnv> = ['NODE_ENV', 'NEXT_PUBLIC_USE_MOCK_API', 'NEXT_PUBLIC_ALLOW_DEMO_AUTH']

function setEnvValue(key: keyof NodeJS.ProcessEnv, value: string | undefined) {
  if (value === undefined) {
    delete process.env[key]
    return
  }

  Object.defineProperty(process.env, key, {
    value,
    configurable: true,
    writable: true,
    enumerable: true,
  })
}

function loadAuthOptions(params: LoadAuthOptionsParams = {}): NextAuthOptions {
  const snapshot: Record<string, string | undefined> = {}
  ENV_KEYS.forEach((key) => {
    snapshot[key] = process.env[key]
  })

  if (params.nodeEnv) {
    setEnvValue('NODE_ENV', params.nodeEnv)
  }

  if (params.env) {
    Object.entries(params.env).forEach(([key, value]) => {
      setEnvValue(key as keyof NodeJS.ProcessEnv, value)
    })
  }

  let authOptions: NextAuthOptions | undefined

  jest.isolateModules(() => {
    if (params.apiClientMock) {
      jest.doMock('@/lib/api-client', () => ({
        apiClient: params.apiClientMock,
      }))
    }

    const mod = require('@/lib/auth/options') as { authOptions: NextAuthOptions }
    authOptions = mod.authOptions
  })

  if (params.apiClientMock) {
    jest.dontMock('@/lib/api-client')
  }

  ENV_KEYS.forEach((key) => {
    setEnvValue(key, snapshot[key])
  })

  if (!authOptions) {
    throw new Error('Failed to load auth options')
  }

  return authOptions
}

function getCredentialsProvider(options: NextAuthOptions): any {
  const provider = options.providers.find((p: any) => p.id === 'credentials')
  if (!provider) throw new Error('Credentials provider missing')
  return provider
}

describe('Session/Cookie security (NextAuth)', () => {
  afterEach(() => {
    jest.clearAllMocks()
  })

  it('uses secure cookies in production', () => {
    const authOptions = loadAuthOptions({ nodeEnv: 'production' })
    expect(authOptions.useSecureCookies).toBe(true)
  })

  it('sets shorter maxAge for admin sessions and rotates on role change', async () => {
    const authOptions = loadAuthOptions()

    const token: any = { role: 'publisher' }
    const session: any = { user: { name: 'X' } }

    const publisherSession = await (authOptions.callbacks as any).session({ session, token })
    expect(publisherSession.maxAge).toBe(24 * 60 * 60)

    await (authOptions.callbacks as any).jwt({ token, user: undefined, trigger: 'update', session: { role: 'admin' } })
    const adminSession = await (authOptions.callbacks as any).session({ session, token })
    expect(adminSession.maxAge).toBe(4 * 60 * 60)
    expect(typeof adminSession.sessionVersion).toBe('number')
    expect(adminSession.elevatedAt === undefined || typeof adminSession.elevatedAt === 'number').toBe(true)
  })

  it('authorizes credentials via backend login when not in mock mode', async () => {
    const post = jest.fn().mockResolvedValue({
      data: {
        user: {
          id: 'user-1',
          email: 'ops@example.com',
          name: 'Ops User',
          role: 'admin',
          publisherId: 'pub_123',
        },
        token: 'jwt_admin_token',
      },
    })

    const authOptions = loadAuthOptions({ apiClientMock: { post } })
    const provider = getCredentialsProvider(authOptions)

    const user = await provider.authorize({ email: 'ops@example.com', password: 'hunter2' })

    expect(post).toHaveBeenCalledWith('/auth/login', {
      email: 'ops@example.com',
      password: 'hunter2',
    })
    expect(user).toMatchObject({ email: 'ops@example.com', role: 'admin', accessToken: 'jwt_admin_token' })
  })

  it('honors demo credentials only when mock auth is enabled', async () => {
    const post = jest.fn()
    const authOptions = loadAuthOptions({
      nodeEnv: 'development',
      env: { NEXT_PUBLIC_USE_MOCK_API: 'true', NEXT_PUBLIC_ALLOW_DEMO_AUTH: 'true' },
      apiClientMock: { post },
    })
    const provider = getCredentialsProvider(authOptions)

    const demoUser = await provider.authorize({ email: 'demo@acme.com', password: 'anything' })
    const realUser = await provider.authorize({ email: 'ops@acme.com', password: 'anything' })

    expect(post).not.toHaveBeenCalled()
    expect(demoUser).toMatchObject({ email: 'demo@acme.com', role: 'publisher', accessToken: 'mock_token_123' })
    expect(realUser).toBeNull()
  })

  it('returns null and logs when backend login fails', async () => {
    const error = new Error('network down')
    const post = jest.fn().mockRejectedValue(error)
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => {})

    const authOptions = loadAuthOptions({ apiClientMock: { post } })
    const provider = getCredentialsProvider(authOptions)

    const user = await provider.authorize({ email: 'ops@example.com', password: 'secret' })

    expect(user).toBeNull()
    expect(consoleSpy).toHaveBeenCalled()
    consoleSpy.mockRestore()
  })
})

describe('api client session protections', () => {
  afterEach(() => {
    mockedGetCsrfToken.mockReset()
    mockedReadXsrfCookie.mockReset()
  })

  it('attaches CSRF tokens to mutating requests and caches API-fetched token', async () => {
    const originalAdapter = apiClient.defaults.adapter
    const capturedConfigs: any[] = []
    const adapter = jest.fn(async (config) => {
      capturedConfigs.push(config)
      return {
        data: {},
        status: 200,
        statusText: 'OK',
        headers: {},
        config,
        request: {},
      }
    })

    apiClient.defaults.adapter = adapter as any

    // The new implementation always fetches from API and caches the token
    // (not the cookie value). Both requests should use the same fetched token.
    mockedGetCsrfToken.mockResolvedValue('fetched-token')

    await apiClient.post('/secure/action', { foo: 'bar' })
    await apiClient.post('/secure/again', { fizz: 'buzz' })

    // Token is fetched once and cached for both requests
    expect(mockedGetCsrfToken).toHaveBeenCalledTimes(1)
    expect(adapter).toHaveBeenCalledTimes(2)
    expect(capturedConfigs[0].headers?.['X-CSRF-Token']).toBe('fetched-token')
    expect(capturedConfigs[1].headers?.['X-CSRF-Token']).toBe('fetched-token')

    apiClient.defaults.adapter = originalAdapter
  })

  it('emits an unauthorized browser event when the backend returns 401', async () => {
    const originalAdapter = apiClient.defaults.adapter
    const adapter = jest.fn(async (config) => {
      throw new AxiosError('Unauthorized', undefined, config, {}, {
        status: 401,
        statusText: 'Unauthorized',
        headers: {},
        config,
        data: {},
      })
    })

    apiClient.defaults.adapter = adapter as any

    const eventDetailPromise = new Promise<any>((resolve) => {
      const handler = (event: Event) => {
        window.removeEventListener(AUTH_UNAUTHORIZED_EVENT, handler as EventListener)
        resolve((event as CustomEvent).detail)
      }
      window.addEventListener(AUTH_UNAUTHORIZED_EVENT, handler as EventListener)
    })

    await expect(apiClient.get('/secure/resource')).rejects.toThrow('Unauthorized')
    const detail = await eventDetailPromise

    expect(detail.status).toBe(401)
    expect(detail.url).toContain('/secure/resource')

    apiClient.defaults.adapter = originalAdapter
  })
})
