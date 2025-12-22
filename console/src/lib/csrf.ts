// CSRF helpers for cookie-based sessions

export const XSRF_COOKIE_NAME = process.env.NEXT_PUBLIC_CSRF_COOKIE_NAME || 'XSRF-TOKEN'

export class CsrfFetchError extends Error {
  status?: number

  constructor(message: string, status?: number, options?: ErrorOptions) {
    super(message, options)
    this.name = 'CsrfFetchError'
    this.status = status
  }
}

type GetCsrfTokenOptions = {
  baseUrl?: string
  signal?: AbortSignal
}

const AUTH_CSRF_PATH = '/auth/csrf'
const DEFAULT_BACKEND_PREFIX = '/api/v1'
const USE_MOCK_API = process.env.NEXT_PUBLIC_USE_MOCK_API === 'true' && process.env.NODE_ENV !== 'production'

export function readXsrfCookie(): string | null {
  if (typeof document === 'undefined') return null
  const name = XSRF_COOKIE_NAME + '='
  const parts = document.cookie.split(';')
  for (let c of parts) {
    c = c.trim()
    if (c.startsWith(name)) return decodeURIComponent(c.substring(name.length))
  }
  return null
}

function resolveCsrfUrl(baseUrl?: string): string {
  const normalizedBase = baseUrl?.replace(/\/$/, '')
  if (normalizedBase) return `${normalizedBase}${AUTH_CSRF_PATH}`

  if (USE_MOCK_API) {
    return `/api/mock?endpoint=csrf`
  }

  const envBase = process.env.NEXT_PUBLIC_API_URL?.replace(/\/$/, '')
  if (envBase) {
    return `${envBase}${AUTH_CSRF_PATH}`
  }

  if (typeof window !== 'undefined') {
    // In the browser, prefer the backend CSRF endpoint (proxied via Next.js)
    return `${DEFAULT_BACKEND_PREFIX}${AUTH_CSRF_PATH}`
  }

  throw new CsrfFetchError('Unable to resolve CSRF endpoint URL. Set NEXT_PUBLIC_API_URL for server-side calls.')
}

export async function getCsrfToken(options?: GetCsrfTokenOptions): Promise<string> {
  const url = resolveCsrfUrl(options?.baseUrl)
  let response: Response
  try {
    response = await fetch(url, {
      credentials: 'include',
      headers: { Accept: 'application/json' },
      signal: options?.signal,
    })
  } catch (error) {
    throw new CsrfFetchError('Unable to reach CSRF endpoint', undefined, { cause: error })
  }

  if (!response.ok) {
    throw new CsrfFetchError(`CSRF endpoint responded with ${response.status}`, response.status)
  }

  let data: any = null
  try {
    data = await response.json()
  } catch (error) {
    throw new CsrfFetchError('Failed to parse CSRF response payload', response.status, { cause: error })
  }

  const token = data?.token || data?.csrfToken || readXsrfCookie()
  if (!token) {
    throw new CsrfFetchError('CSRF token missing in response payload', response.status)
  }

  return token
}
