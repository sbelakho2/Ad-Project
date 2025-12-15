import { NextResponse } from 'next/server'
import { cookies } from 'next/headers'
import { getOperatorCookieName, getOperatorCookieOptions } from '@/lib/operatorSession'

export async function POST() {
  const cookieStore = await cookies()
  cookieStore.set(getOperatorCookieName(), '', {
    ...getOperatorCookieOptions(),
    maxAge: 0,
  })

  return NextResponse.json({ ok: true })
}
