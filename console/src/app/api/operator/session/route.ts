import { NextResponse } from 'next/server'
import { cookies } from 'next/headers'
import { getOperatorCookieName, verifyOperatorSessionToken } from '@/lib/operatorSession'

export async function GET() {
  const cookieStore = await cookies()
  const token = cookieStore.get(getOperatorCookieName())?.value
  if (!token) {
    return NextResponse.json({ ok: false }, { status: 401 })
  }

  const payload = verifyOperatorSessionToken(token)
  if (!payload) {
    return NextResponse.json({ ok: false }, { status: 401 })
  }

  return NextResponse.json({ ok: true, identifier: payload.identifier, exp: payload.exp })
}
