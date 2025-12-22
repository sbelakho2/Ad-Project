import axios, { AxiosInstance, AxiosError, AxiosHeaders, InternalAxiosRequestConfig } from 'axios'
import { getCsrfToken, readXsrfCookie } from './csrf'
import { getSession } from 'next-auth/react'

// Mock API is never enabled in production
const USE_MOCK_API = process.env.NEXT_PUBLIC_USE_MOCK_API === 'true' && process.env.NODE_ENV !== 'production'
const TENANT_HEADER = process.env.NEXT_PUBLIC_TENANT_HEADER || 'X-Publisher-Id'

const appendApiVersion = (value: string, defaultSuffix = '/api/v1') => {
  const normalized = value.replace(/\/+$/, '')
  if (/\/api\/v\d+$/i.test(normalized) || /\/v\d+$/i.test(normalized)) {
    return normalized
  }
  if (/\/api$/i.test(normalized)) {
    return `${normalized}/v1`
  }
  return `${normalized}${defaultSuffix}`
}

const normalizeOrigin = (value: string) => value.trim().replace(/\/+$/, '')

const getServerOriginFallback = () => {
  const nextAuthUrl = process.env.NEXTAUTH_URL || ''
  if (nextAuthUrl) return normalizeOrigin(nextAuthUrl)

  const vercelUrl = process.env.VERCEL_URL || ''
  if (vercelUrl) return `https://${normalizeOrigin(vercelUrl)}`

  const port = process.env.PORT || '3000'
  return `http://localhost:${port}`
}

const normalizeBase = (candidate: string | undefined, fallbackHost: string) => {
  const base = (candidate && candidate.trim().length > 0 ? candidate.trim() : fallbackHost)
  return appendApiVersion(base)
}

// In production browsers, always use same-origin relative URLs to avoid CORS and
// cross-site cookie/SameSite issues. The Next.js API routes proxy to the backend.
//
// IMPORTANT: Do not rely on NEXT_PUBLIC_API_URL in the browser in production.
// If it is set to an absolute api.* host, requests become cross-origin and will
// frequently fail (preflight/CORS, SameSite cookies), which breaks nav + analytics.
const IS_PRODUCTION = process.env.NODE_ENV === 'production'
const IS_BROWSER = typeof window !== 'undefined'

// NOTE: On the server (NextAuth callbacks, server actions), we may need an absolute
// backend URL. Prefer API_URL (server-only) but fall back to NEXT_PUBLIC_API_URL.
const CORE_HOST_SERVER = process.env.API_URL || process.env.NEXT_PUBLIC_API_URL || ''
const CORE_HOST_BROWSER = process.env.NEXT_PUBLIC_API_URL || ''

const getDefaultBase = () => {
  if (USE_MOCK_API) return '/api/mock'

  // Production browser must go through the same-origin proxy.
  if (IS_PRODUCTION && IS_BROWSER) return '/api/v1'

  // Server-side code can call the backend directly.
  if (!IS_BROWSER && CORE_HOST_SERVER) return normalizeBase(CORE_HOST_SERVER, '')

  // Server-side code must have an absolute URL for axios. If no backend host is configured,
  // use the Next.js same-origin proxy via NEXTAUTH_URL/VERCEL_URL/localhost.
  if (!IS_BROWSER) {
    return `${getServerOriginFallback()}/api/v1`
  }

  // Non-production browser can use an explicit host if configured.
  if (IS_BROWSER && CORE_HOST_BROWSER) return normalizeBase(CORE_HOST_BROWSER, '')

  return '/api/v1'
}

const BASE_URL = getDefaultBase()

// Keep analytics/fraud on the same origin in production browsers as well.
const FRAUD_API_URL = USE_MOCK_API
  ? '/api/mock'
  : IS_PRODUCTION && IS_BROWSER
    ? BASE_URL
    : process.env.NEXT_PUBLIC_FRAUD_API_URL
      ? normalizeBase(process.env.NEXT_PUBLIC_FRAUD_API_URL, '')
      : BASE_URL

const ANALYTICS_API_URL = USE_MOCK_API
  ? '/api/mock'
  : IS_PRODUCTION && IS_BROWSER
    ? BASE_URL
    : process.env.NEXT_PUBLIC_ANALYTICS_API_URL
      ? normalizeBase(process.env.NEXT_PUBLIC_ANALYTICS_API_URL, '')
      : BASE_URL

// Create axios instances
export const apiClient: AxiosInstance = axios.create({
  baseURL: BASE_URL,
  timeout: 10000,
  headers: {
    'Content-Type': 'application/json',
  },
  withCredentials: true,
})

export const fraudApiClient: AxiosInstance = axios.create({
  baseURL: FRAUD_API_URL,
  timeout: 10000,
  headers: {
    'Content-Type': 'application/json',
  },
  withCredentials: true,
})

export const analyticsApiClient: AxiosInstance = axios.create({
  baseURL: ANALYTICS_API_URL,
  timeout: 10000,
  headers: {
    'Content-Type': 'application/json',
  },
  withCredentials: true,
})

const MUTATING_METHODS = new Set(['post', 'put', 'patch', 'delete'])

let inflightCsrfRequest: Promise<string | null> | null = null
let activeTenantId: string | null = null

export function setActiveTenantId(nextTenantId: string | null) {
  activeTenantId = typeof nextTenantId === 'string' && nextTenantId.trim().length > 0 ? nextTenantId : null
}

async function ensureBrowserCsrfToken(): Promise<string | null> {
  if (typeof window === 'undefined') return null
  const existing = readXsrfCookie()
  if (existing) return existing

  if (!inflightCsrfRequest) {
    inflightCsrfRequest = getCsrfToken().catch(() => null).finally(() => {
      inflightCsrfRequest = null
    })
  }

  return inflightCsrfRequest
}

const requestInterceptor = async (config: InternalAxiosRequestConfig) => {
  const method = (config.method || 'get').toLowerCase()
  
  // Add Bearer token from NextAuth session for API authentication
  if (typeof window !== 'undefined') {
    try {
      const session = await getSession()
      const accessToken = (session as any)?.accessToken
      if (accessToken) {
        const headers = AxiosHeaders.from(config.headers || {})
        headers.set('Authorization', `Bearer ${accessToken}`)
        config.headers = headers
      }
    } catch {
      // Session fetch failed, continue without token
    }
  }

  if (MUTATING_METHODS.has(method)) {
    const token = await ensureBrowserCsrfToken()
    if (token) {
      const headers = AxiosHeaders.from(config.headers || {})
      headers.set('X-CSRF-Token', token)
      config.headers = headers
    }
  }

  if (activeTenantId) {
    const headers = AxiosHeaders.from(config.headers || {})
    headers.set(TENANT_HEADER, activeTenantId)
    config.headers = headers
  }
  return config
}

export const AUTH_UNAUTHORIZED_EVENT = 'apex:auth:unauthorized'

type UnauthorizedHandler = (error: AxiosError) => void

let unauthorizedHandler: UnauthorizedHandler | null = null

export function setUnauthorizedHandler(handler: UnauthorizedHandler | null) {
  unauthorizedHandler = handler
}

function emitUnauthorized(error: AxiosError) {
  if (unauthorizedHandler) {
    unauthorizedHandler(error)
    return
  }

  if (typeof window !== 'undefined') {
    window.dispatchEvent(
      new CustomEvent(AUTH_UNAUTHORIZED_EVENT, {
        detail: {
          status: error.response?.status || null,
          url: error.config?.url,
        },
      })
    )
  }
}

// Response interceptor - handle errors
const responseErrorInterceptor = async (error: AxiosError) => {
  if (error.response?.status === 401) {
    emitUnauthorized(error)
  }
  return Promise.reject(error)
}

// Apply interceptors
apiClient.interceptors.request.use(requestInterceptor)
apiClient.interceptors.response.use((response) => response, responseErrorInterceptor)

fraudApiClient.interceptors.request.use(requestInterceptor)
fraudApiClient.interceptors.response.use((response) => response, responseErrorInterceptor)

analyticsApiClient.interceptors.request.use(requestInterceptor)
analyticsApiClient.interceptors.response.use((response) => response, responseErrorInterceptor)

// API helper functions
export const handleApiError = (error: unknown): string => {
  if (axios.isAxiosError(error)) {
    return error.response?.data?.message || error.message
  }
  const maybeResponse = (error as any)?.response?.data?.message
  if (maybeResponse) return maybeResponse
  if (error instanceof Error) return error.message
  return 'An unexpected error occurred'
}

// Cookie-based auth; no localStorage usage
