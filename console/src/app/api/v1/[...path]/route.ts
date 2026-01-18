import { NextRequest } from 'next/server'

const normalizeApiBase = (value: string) => {
  const trimmed = value.trim().replace(/\/+$/, '')
  if (/\/api\/v\d+$/i.test(trimmed)) return trimmed
  if (/\/api$/i.test(trimmed)) return `${trimmed}/v1`
  return `${trimmed}/api/v1`
}

function inferApiBaseFromHost(host: string, proto: string): string {
  const normalizedHost = host.trim().toLowerCase()
  if (!normalizedHost) throw new Error('Unable to infer API base URL: missing Host header')

  // Production default: console.<root> -> api.<root>
  if (normalizedHost.startsWith('console.')) {
    return `${proto}//api.${normalizedHost.substring('console.'.length)}`
  }

  // Allow direct api host in case this route is used in other environments
  if (normalizedHost.startsWith('api.')) {
    return `${proto}//${normalizedHost}`
  }

  throw new Error(
    `Unable to infer API base URL from host '${host}'. Set NEXT_PUBLIC_API_URL (recommended) or API_URL.`
  )
}

function getBackendBaseUrl(request: NextRequest): string {
  const env = process.env.NEXT_PUBLIC_API_URL || process.env.API_URL || ''
  if (env) return normalizeApiBase(env)

  const incoming = new URL(request.url)
  const proto = incoming.protocol.replace(':', '') || 'https'
  const host = request.headers.get('host') || ''
  return normalizeApiBase(inferApiBaseFromHost(host, proto))
}

const HOP_BY_HOP_HEADERS = new Set([
  'connection',
  'keep-alive',
  'proxy-authenticate',
  'proxy-authorization',
  'te',
  'trailer',
  'transfer-encoding',
  'upgrade',
])

function filterRequestHeaders(headers: Headers): Headers {
  const out = new Headers()
  for (const [key, value] of headers.entries()) {
    const lower = key.toLowerCase()
    if (HOP_BY_HOP_HEADERS.has(lower)) continue

    // If we forward a body, fetch must compute framing headers.
    // Keeping an incoming Content-Length can cause the upstream to see an empty body.
    if (lower === 'content-length') continue
    // Let fetch negotiate encoding for the upstream.
    if (lower === 'accept-encoding') continue

    // Let fetch set this automatically based on the new URL
    if (lower === 'host') continue

    // Avoid leaking Next.js internals upstream
    if (lower.startsWith('x-middleware-')) continue

    out.set(key, value)
  }
  return out
}

async function proxy(request: NextRequest, params: Promise<{ path?: string[] }>) {
  const { path = [] } = await params
  const backendBase = getBackendBaseUrl(request)
  const upstreamUrl = new URL(`${backendBase}/${path.map(encodeURIComponent).join('/')}`)

  // Preserve query string
  const incoming = new URL(request.url)
  upstreamUrl.search = incoming.search

  const headers = filterRequestHeaders(request.headers)

  // Ensure backend sees correct forwarding info
  headers.set('X-Forwarded-Host', request.headers.get('host') || '')
  headers.set('X-Forwarded-Proto', incoming.protocol.replace(':', ''))

  const init: RequestInit = {
    method: request.method,
    headers,
    redirect: 'manual',
  }

  // Only forward body for methods that allow one
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    init.body = await request.arrayBuffer()
  }

  const upstreamResponse = await fetch(upstreamUrl, init)

  const responseHeaders = new Headers()
  const maybeHeadersWithSetCookie = upstreamResponse.headers as any
  const setCookies =
    typeof maybeHeadersWithSetCookie?.getSetCookie === 'function'
      ? (maybeHeadersWithSetCookie.getSetCookie as (this: Headers) => string[]).call(upstreamResponse.headers)
      : null

  upstreamResponse.headers.forEach((value, key) => {
    const lower = key.toLowerCase()
    if (HOP_BY_HOP_HEADERS.has(lower)) return
    if (lower === 'set-cookie') return
    // The fetch API automatically decompresses the response body.
    // Do NOT forward content-encoding as the body is now uncompressed.
    // This prevents Caddy from double-compressing the response.
    if (lower === 'content-encoding') return
    // Content-length may also be wrong after decompression
    if (lower === 'content-length') return
    responseHeaders.set(key, value)
  })

  if (setCookies?.length) {
    for (const cookie of setCookies) {
      responseHeaders.append('set-cookie', cookie)
    }
  } else {
    const single = upstreamResponse.headers.get('set-cookie')
    if (single) responseHeaders.set('set-cookie', single)
  }

  return new Response(upstreamResponse.body, {
    status: upstreamResponse.status,
    headers: responseHeaders,
  })
}

export async function GET(request: NextRequest, context: { params: Promise<{ path?: string[] }> }) {
  return proxy(request, context.params)
}

export async function POST(request: NextRequest, context: { params: Promise<{ path?: string[] }> }) {
  return proxy(request, context.params)
}

export async function PUT(request: NextRequest, context: { params: Promise<{ path?: string[] }> }) {
  return proxy(request, context.params)
}

export async function PATCH(request: NextRequest, context: { params: Promise<{ path?: string[] }> }) {
  return proxy(request, context.params)
}

export async function DELETE(request: NextRequest, context: { params: Promise<{ path?: string[] }> }) {
  return proxy(request, context.params)
}

export async function OPTIONS(request: NextRequest, context: { params: Promise<{ path?: string[] }> }) {
  return proxy(request, context.params)
}
