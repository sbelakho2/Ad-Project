import { jwtVerify } from 'jose';
import type { NextRequest } from 'next/server';
import { NextResponse } from 'next/server';

const JWT_SECRET = new TextEncoder().encode(
  process.env.JWT_SECRET || 'your-secret-key-change-in-production'
);

// Paths that require authentication (prefix matches)
const protectedPaths = ['/dashboard', '/settings', '/api/internal', '/api/auth'];
// Public auth endpoints that must remain accessible without a session
const publicAuthPaths = ['/api/auth/login', '/api/auth/signup', '/api/auth/me', '/api/auth/logout'];

// Paths that should redirect to dashboard if already authenticated
const authPaths = ['/signin', '/signup'];

export async function middleware(request: NextRequest) {
  const hostname = request.headers.get('host') || '';
  const { pathname } = request.nextUrl;

  // Host-based routing for marketing subdomains.
  // These subdomains are served by the same Next.js app; we rewrite paths so they resolve.
  // - docs.apexmediation.ee/foo -> /docs/foo
  // - status.apexmediation.ee/foo -> /status/foo
  if (!pathname.startsWith('/api')) {
    const isDocsHost = hostname.startsWith('docs.');
    const isStatusHost = hostname.startsWith('status.');

    if (isDocsHost) {
      const nextPath = pathname === '/' ? '/docs' : pathname.startsWith('/docs') ? pathname : `/docs${pathname}`;
      const url = request.nextUrl.clone();
      url.pathname = nextPath;
      return NextResponse.rewrite(url);
    }

    if (isStatusHost) {
      const nextPath = pathname === '/' ? '/status' : pathname.startsWith('/status') ? pathname : `/status${pathname}`;
      const url = request.nextUrl.clone();
      url.pathname = nextPath;
      return NextResponse.rewrite(url);
    }
  }

  // Get token from cookie
  const token = request.cookies.get('session')?.value;

  // Verify token
  let isAuthenticated = false;
  if (token) {
    try {
      await jwtVerify(token, JWT_SECRET);
      isAuthenticated = true;
    } catch (error) {
      // Token is invalid or expired
      isAuthenticated = false;
    }
  }

  // Check if path requires authentication
  const isProtectedPath = protectedPaths.some(path => pathname.startsWith(path));
  const isPublicAuthPath = publicAuthPaths.some(path => pathname.startsWith(path));
  const isAuthPath = authPaths.some(path => pathname.startsWith(path));

  // Redirect to signin if accessing protected path without authentication
  if (isProtectedPath && !isAuthenticated && !isPublicAuthPath) {
    const url = request.nextUrl.clone();
    url.pathname = '/signin';
    url.searchParams.set('redirect', pathname);
    url.searchParams.set('reason', token ? 'invalid_session' : 'unauthenticated');
    const resp = NextResponse.redirect(url);
    // Clear potentially invalid cookie to avoid loops
    resp.cookies.delete('session');
    return resp;
  }

  // Redirect to dashboard if accessing auth pages while authenticated
  if (isAuthPath && isAuthenticated) {
    const url = request.nextUrl.clone();
    url.pathname = '/dashboard';
    return NextResponse.redirect(url);
  }

  return NextResponse.next();
}

export const config = {
  matcher: [
    /*
     * Match all request paths except:
     * - _next/static (static files)
     * - _next/image (image optimization files)
     * - favicon.ico (favicon file)
     * - public folder
     *
     * Include /api/internal and /api/auth for protection, but skip other public API routes
     */
    '/((?!_next/static|_next/image|favicon.ico|public).*)',
  ],
};
