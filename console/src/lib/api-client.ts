import axios, { AxiosInstance, AxiosError, AxiosHeaders, InternalAxiosRequestConfig } from 'axios'
import { getCsrfToken, readXsrfCookie } from './csrf'

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

const normalizeBase = (candidate: string | undefined, fallbackHost: string) => {
  const base = (candidate && candidate.trim().length > 0 ? candidate.trim() : fallbackHost)
  return appendApiVersion(base)
}

// In production, use relative URLs to avoid CORS and localhost issues
// The Next.js API routes proxy to the backend
const isServer = typeof window === 'undefined'
const CORE_HOST = process.env.NEXT_PUBLIC_API_URL || ''

// Use relative paths when no explicit host is set - Next.js will proxy these
const getDefaultBase = () => {
  if (USE_MOCK_API) return '/api/mock'
  if (CORE_HOST) return normalizeBase(CORE_HOST, '')
  // In browser, use relative URL; on server during SSR, skip external calls
  return '/api/v1'
}

const BASE_URL = getDefaultBase()
const FRAUD_API_URL = USE_MOCK_API
  ? '/api/mock'
  : process.env.NEXT_PUBLIC_FRAUD_API_URL
    ? normalizeBase(process.env.NEXT_PUBLIC_FRAUD_API_URL, '')
    : BASE_URL
const ANALYTICS_API_URL = USE_MOCK_API
  ? '/api/mock'
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
