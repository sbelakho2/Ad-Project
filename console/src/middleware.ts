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
  const pathname = request.nextUrl.pathname

  // Admin subdomain handling
  if (isAdminSubdomain) {
    // Root of admin subdomain -> redirect to /admin/overview (or /admin/login if not authenticated)
    if (pathname === '/') {
      return NextResponse.redirect(new URL('/admin/overview', request.url))
    }

    // /login on admin subdomain -> redirect to operator login
    if (pathname === '/login') {
      return NextResponse.redirect(new URL('/admin/login', request.url))
    }

    // Any customer console routes on admin subdomain -> redirect to admin
    const customerRoutes = ['/dashboard', '/billing', '/payouts', '/settings', '/transparency', '/signup']
    if (customerRoutes.some(route => pathname.startsWith(route))) {
      return NextResponse.redirect(new URL('/admin/overview', request.url))
    }
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
