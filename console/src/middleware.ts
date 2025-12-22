import { NextResponse } from 'next/server'
import type { NextRequest } from 'next/server'

/**
 * Middleware to handle admin subdomain routing
 * 
 * When users access admin.apexmediation.ee, they should be routed to
 * the operator control plane, not the customer console.
 */
export function middleware(request: NextRequest) {
  const hostname = request.headers.get('host') || ''
  const isAdminSubdomain = hostname.startsWith('admin.')

  const { pathname } = request.nextUrl

  // Hard separation:
  // The admin hostname must never be served by customer console routes.
  // Only /admin/* UI routes are allowed on admin.*
  if (isAdminSubdomain) {
    if (pathname.startsWith('/admin')) {
      return NextResponse.next()
    }

    const url = request.nextUrl.clone()
    url.pathname = '/admin/login'
    url.search = ''
    return NextResponse.redirect(url)
  }

  return NextResponse.next()
}

export const config = {
  matcher: [
    /*
     * Match all request paths except for the ones starting with:
     * - api (API routes)
     * - _next/static (static files)
     * - _next/image (image optimization files)
     * - favicon.ico (favicon file)
     * - public folder
     */
    '/((?!api|_next/static|_next/image|favicon.ico|public).*)',
  ],
}
