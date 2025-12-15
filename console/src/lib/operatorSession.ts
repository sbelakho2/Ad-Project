import crypto from 'crypto'

const COOKIE_NAME = 'apex_operator_session'

type OperatorSessionPayload = {
  identifier: string
  exp: number // unix seconds
}

function getOperatorSecret(): string | null {
  return process.env.OPERATOR_SECRET || process.env.NEXT_PUBLIC_OPERATOR_SECRET || null
}

function base64urlEncode(input: Buffer | string): string {
  const buf = typeof input === 'string' ? Buffer.from(input, 'utf8') : input
  return buf
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '')
}

function base64urlDecodeToBuffer(input: string): Buffer {
  const normalized = input.replace(/-/g, '+').replace(/_/g, '/')
  const padded = normalized + '==='.slice((normalized.length + 3) % 4)
  return Buffer.from(padded, 'base64')
}

function sign(data: string, secret: string): string {
  const signature = crypto.createHmac('sha256', secret).update(data).digest()
  return base64urlEncode(signature)
}

export function getOperatorCookieName() {
  return COOKIE_NAME
}

export function buildOperatorSessionToken(params: {
  identifier: string
  ttlSeconds: number
}): string {
  const secret = getOperatorSecret()
  if (!secret) {
    throw new Error('OPERATOR_SECRET not configured')
  }

  const nowSeconds = Math.floor(Date.now() / 1000)
  const payload: OperatorSessionPayload = {
    identifier: params.identifier,
    exp: nowSeconds + params.ttlSeconds,
  }

  const payloadPart = base64urlEncode(JSON.stringify(payload))
  const sigPart = sign(payloadPart, secret)
  return `${payloadPart}.${sigPart}`
}

export function verifyOperatorSessionToken(token: string): OperatorSessionPayload | null {
  const secret = getOperatorSecret()
  if (!secret) return null

  const [payloadPart, sigPart] = token.split('.')
  if (!payloadPart || !sigPart) return null

  const expectedSig = sign(payloadPart, secret)

  try {
    const a = Buffer.from(expectedSig, 'utf8')
    const b = Buffer.from(sigPart, 'utf8')
    if (a.length !== b.length) return null
    if (!crypto.timingSafeEqual(a, b)) return null
  } catch {
    return null
  }

  try {
    const payloadJson = base64urlDecodeToBuffer(payloadPart).toString('utf8')
    const payload = JSON.parse(payloadJson) as OperatorSessionPayload
    if (!payload?.identifier || typeof payload.exp !== 'number') return null

    const nowSeconds = Math.floor(Date.now() / 1000)
    if (payload.exp <= nowSeconds) return null

    return payload
  } catch {
    return null
  }
}

export function getOperatorCookieOptions() {
  const isProd = process.env.NODE_ENV === 'production'
  return {
    httpOnly: true,
    secure: isProd,
    sameSite: 'lax' as const,
    path: '/',
  }
}
