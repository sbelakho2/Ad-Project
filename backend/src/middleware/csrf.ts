import { NextFunction, Request, Response } from 'express';
import csurf from 'csurf';

const bool = (v: string | undefined, dflt = false) => (v == null ? dflt : v === '1' || v.toLowerCase?.() === 'true');

// Resolve SameSite setting safely
function resolveSameSite(): 'lax' | 'strict' | 'none' {
  const v = (process.env.COOKIE_SAMESITE || 'lax').toLowerCase();
  return v === 'none' ? 'none' : v === 'strict' ? 'strict' : 'lax';
}

// Double-submit cookie strategy: httpOnly auth cookies + readable XSRF-TOKEN cookie
const baseCsurf = csurf({
  cookie: {
    key: process.env.CSRF_COOKIE_NAME || 'XSRF-TOKEN',
    httpOnly: false, // must be readable by browser JS
    sameSite: resolveSameSite(),
    secure: bool(process.env.COOKIE_SECURE, process.env.NODE_ENV === 'production'),
    domain: process.env.COOKIE_DOMAIN || undefined,
    path: '/',
  },
  value: (req: Request) => {
    const hdr = req.headers['x-csrf-token'];
    if (typeof hdr === 'string' && hdr) return hdr;
    const body = (req as unknown as { body?: { _csrf?: string } }).body;
    return body?._csrf || '';
  },
});

// Wrapper that ignores CSRF for specific auth acquisition endpoints (login/register/refresh)
// and SDK/RTB endpoints that use API key authentication (not browser sessions)
export function csrfProtection(req: Request, res: Response, next: NextFunction) {
  const path = req.path || '';
  const method = req.method.toUpperCase();
  const apiVersion = process.env.API_VERSION || 'v1';
  const authAcquire = new RegExp(`^/api/${apiVersion}/auth/(login|register|refresh)$`);
  const flagsPath = new RegExp(`^/api/${apiVersion}/flags`);
  const rtbPath = new RegExp(`^/api/${apiVersion}/rtb/`);
  const webhooksPath = new RegExp(`^/api/${apiVersion}/webhooks/`);
  const analyticsEventsPath = new RegExp(`^/api/${apiVersion}/analytics/events/`);
  const migrationAssignPath = new RegExp(`^/api/${apiVersion}/migration/assign$`);
  // Exempt CSRF for auth acquisition endpoints, ops-only feature flag toggles, and SDK/RTB endpoints
  if (method === 'POST' && (authAcquire.test(path) || flagsPath.test(path))) {
    return next();
  }
  // RTB/bidding endpoints use SDK key auth (not session cookies), exempt from CSRF
  if (rtbPath.test(path)) {
    return next();
  }
  // Webhooks are server-to-server, exempt from CSRF
  if (webhooksPath.test(path)) {
    return next();
  }
  // Analytics event ingestion from SDKs - no browser session, exempt from CSRF
  if (analyticsEventsPath.test(path)) {
    return next();
  }
  // Migration /assign endpoint uses SDK secret auth, exempt from CSRF
  if (migrationAssignPath.test(path)) {
    return next();
  }
  return (baseCsurf as unknown as (req: Request, res: Response, next: NextFunction) => void)(req, res, next);
}

// Helper route handler to set token cookie explicitly
export function issueCsrfToken(req: Request, res: Response, _next: NextFunction) {
  // When using csurf, req.csrfToken() will set the cookie via the configured cookie options
  const maybeTokenFn = (req as unknown as { csrfToken?: () => string }).csrfToken;
  const token = typeof maybeTokenFn === 'function' ? maybeTokenFn() : '';
  res.json({ success: true, token });
}

export default csrfProtection;
