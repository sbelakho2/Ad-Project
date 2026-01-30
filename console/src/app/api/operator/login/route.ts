import { NextResponse } from 'next/server'
import { cookies } from 'next/headers'
import { buildOperatorSessionToken, getOperatorCookieName, getOperatorCookieOptions } from '@/lib/operatorSession'

export async function POST(request: Request) {
  let body: unknown
  try {
    body = await request.json()
  } catch {
    return NextResponse.json({ ok: false, error: 'Invalid JSON' }, { status: 400 })
  }

  const { secret, identifier } = (body ?? {}) as {
    secret?: string
    identifier?: string
  }

  if (!secret || typeof secret !== 'string') {
    return NextResponse.json({ ok: false, error: 'Missing secret' }, { status: 400 })
  }

  // SECURITY: Only use server-side OPERATOR_SECRET, never NEXT_PUBLIC_*
  // NEXT_PUBLIC_* variables are exposed in client bundles
  const expected = process.env.OPERATOR_SECRET
  if (!expected) {
    return NextResponse.json(
      { ok: false, error: 'Operator login not configured' },
      { status: 503 }
    )
  }

  if (secret !== expected) {
    return NextResponse.json({ ok: false, error: 'Invalid credentials' }, { status: 401 })
  }

  const resolvedIdentifier = (identifier && String(identifier).trim()) || 'operator'

  const token = buildOperatorSessionToken({ identifier: resolvedIdentifier, ttlSeconds: 12 * 60 * 60 })

  const cookieStore = await cookies()
  cookieStore.set(getOperatorCookieName(), token, {
    ...getOperatorCookieOptions(),
    maxAge: 12 * 60 * 60,
  })

  return NextResponse.json({ ok: true, identifier: resolvedIdentifier })
}
