import { NextResponse } from 'next/server'

export const dynamic = 'force-dynamic'

const allowMockApi = process.env.NODE_ENV !== 'production' && process.env.NEXT_PUBLIC_USE_MOCK_API === 'true'

export async function GET() {
  if (!allowMockApi) {
    return NextResponse.json({ error: 'Mock API disabled' }, { status: 404 })
  }

  return NextResponse.json({
    success: true,
    data: {
      billing: true,
      transparency: true,
      migrationStudio: true,
      fraudDetection: true,
      abTesting: true,
    },
  })
}
