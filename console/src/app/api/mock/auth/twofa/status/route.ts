import { NextResponse } from 'next/server'

export async function GET() {
  // Mock 2FA status for deterministic E2E runs.
  return NextResponse.json({
    success: true,
    data: {
      enabled: false,
      verified: false,
    },
  })
}
