import NextAuth from 'next-auth'
import type { NextRequest } from 'next/server'
import { authOptions } from '@/lib/auth/options'
import { 
  checkRateLimit, 
  getRateLimitKey, 
  createRateLimitResponse, 
  applyRateLimitHeaders,
  RATE_LIMITS 
} from '@/lib/rateLimit'

// Wrap NextAuth handler with rate limiting (P1 Security Fix #2)
const nextAuthHandler = NextAuth(authOptions)

async function handleWithRateLimit(
  request: NextRequest,
  handler: typeof nextAuthHandler
) {
  const key = getRateLimitKey(request, 'nextauth')
  const result = checkRateLimit(key, RATE_LIMITS.auth)
  
  if (!result.success) {
    return createRateLimitResponse(result)
  }
  
  // Call the original handler
  const response = await handler(request)
  
  // Add rate limit headers to response
  if (response instanceof Response) {
    const headers = new Headers(response.headers)
    applyRateLimitHeaders(headers, result)
    return new Response(response.body, {
      status: response.status,
      statusText: response.statusText,
      headers,
    })
  }
  
  return response
}

export async function GET(request: NextRequest) {
  return handleWithRateLimit(request, nextAuthHandler)
}

export async function POST(request: NextRequest) {
  return handleWithRateLimit(request, nextAuthHandler)
}
