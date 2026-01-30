import { Request, Response, NextFunction } from 'express';

/**
 * Sets a strong baseline of security headers. Configure CSP via env if needed.
 * Note: Wire this middleware in your Express app before route handlers.
 */
export function securityHeaders(req: Request, res: Response, next: NextFunction) {
  // HSTS (only in production over HTTPS)
  if (process.env.NODE_ENV === 'production') {
    res.setHeader('Strict-Transport-Security', 'max-age=31536000; includeSubDomains; preload');
  }

  // MIME sniffing
  res.setHeader('X-Content-Type-Options', 'nosniff');

  // Referrer policy
  res.setHeader('Referrer-Policy', 'no-referrer');

  // Permissions-Policy (restrict powerful features by default)
  res.setHeader(
    'Permissions-Policy',
    [
      'geolocation=()',
      'microphone=()',
      'camera=()',
      'fullscreen=(self)',
      'payment=()',
    ].join(', ')
  );

  // Basic CSP; allow override via env
  // Production: Remove 'unsafe-eval' to prevent XSS attacks via eval()
  // Development: Allow 'unsafe-eval' for debugging/hot-reload scenarios
  const isProd = process.env.NODE_ENV === 'production';
  const scriptSrc = isProd
    ? "script-src 'self' 'unsafe-inline'"  // Production: No unsafe-eval
    : "script-src 'self' 'unsafe-inline' 'unsafe-eval'"; // Dev: Allow eval
  
  const csp = process.env.CSP_HEADER ||
    [
      "default-src 'self'",
      scriptSrc,
      "style-src 'self' 'unsafe-inline'",
      "img-src 'self' data:",
      "connect-src 'self'",
      "font-src 'self' data:",
      "frame-ancestors 'none'",
      "base-uri 'self'",
      "form-action 'self'",
    ].join('; ');
  res.setHeader('Content-Security-Policy', csp);

  next();
}

export default securityHeaders;
