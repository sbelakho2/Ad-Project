import { NextResponse } from 'next/server'

export const dynamic = 'force-dynamic'

const allowMockApi = process.env.NODE_ENV !== 'production' && process.env.NEXT_PUBLIC_USE_MOCK_API === 'true'

export async function GET() {
  if (!allowMockApi) {
    return NextResponse.json({ error: 'Mock API disabled' }, { status: 404 })
  }

  // Mirrors the shape expected by useSession() => apiClient.get('/auth/me')
  return NextResponse.json({
    success: true,
    data: {
      userId: 'usr-mock',
      publisherId: 'pub-test-123',
      email: 'playwright@example.com',
      role: 'admin',
      permissions: ['billing:view', 'billing:manage'],
    },
  })
}
