/**
 * CSRF Protection Middleware
 * 
 * Implements CSRF protection for the control plane admin UI.
 * 
 * Protection mechanisms:
 * 1. Double-submit cookie pattern - CSRF token in both cookie and request header
 * 2. SameSite=Strict cookies - prevents cross-origin requests from sending cookies
 * 3. Origin/Referer validation - ensures requests come from same origin
 * 
 * Usage:
 * - Call generateCsrfToken() to create a new token and set the cookie
 * - Include the token in requests via X-CSRF-Token header
 * - Apply validateCsrfToken middleware to all state-changing routes
 */

import { Request, Response, NextFunction } from 'express';
import crypto from 'crypto';
import { logger, logAudit } from './logger';
import { getClientIP } from './auth';

// CSRF token configuration
export const CSRF_COOKIE_NAME = 'csrf_token';
export const CSRF_HEADER_NAME = 'x-csrf-token';
const CSRF_TOKEN_LENGTH = 32; // 256 bits
const CSRF_TOKEN_TTL_HOURS = 24; // Token validity period

/**
 * Generate a cryptographically secure CSRF token
 */
export function generateCsrfToken(): string {
  return crypto.randomBytes(CSRF_TOKEN_LENGTH).toString('base64url');
}

/**
 * Set CSRF token cookie
 * Called on login and periodically to rotate tokens
 */
export function setCsrfCookie(res: Response, token: string): void {
  res.cookie(CSRF_COOKIE_NAME, token, {
    httpOnly: false, // Must be readable by JavaScript to include in headers
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'strict', // Prevents cross-origin requests from sending cookie
    maxAge: CSRF_TOKEN_TTL_HOURS * 60 * 60 * 1000,
    path: '/',
  });
}

/**
 * Clear CSRF token cookie (on logout)
 */
export function clearCsrfCookie(res: Response): void {
  res.clearCookie(CSRF_COOKIE_NAME, {
    httpOnly: false,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'strict',
    path: '/',
  });
}

/**
 * Validate Origin header matches expected host
 */
function validateOrigin(req: Request): boolean {
  const origin = req.get('Origin');
  const referer = req.get('Referer');
  const host = req.get('Host');
  
  // Must have either Origin or Referer header
  if (!origin && !referer) {
    // Allow requests from same-origin if no Origin/Referer
    // This handles direct API calls from the same server
    return true;
  }
  
  // Extract hostname from Origin or Referer
  const requestOrigin = origin || (referer ? new URL(referer).origin : null);
  if (!requestOrigin) {
    return false;
  }
  
  // Validate against expected host
  try {
    const expectedOrigins = [
      `https://${host}`,
      `http://${host}`, // For development
    ];
    
    // Add configured CORS origin
    const corsOrigin = process.env.CORS_ORIGIN;
    if (corsOrigin) {
      expectedOrigins.push(corsOrigin);
    }
    
    return expectedOrigins.some(expected => requestOrigin === expected);
  } catch {
    return false;
  }
}

/**
 * CSRF validation middleware
 * Apply to all state-changing routes (POST, PUT, DELETE, PATCH)
 */
export function validateCsrfToken(req: Request, res: Response, next: NextFunction): void {
  // Skip CSRF validation for safe methods
  if (['GET', 'HEAD', 'OPTIONS'].includes(req.method)) {
    return next();
  }
  
  const clientIP = getClientIP(req);
  
  // 1. Validate Origin/Referer header
  if (!validateOrigin(req)) {
    logAudit({
      actor: req.operator?.email || 'unknown',
      action: 'csrf_check',
      result: 'denied',
      reason: 'Invalid Origin/Referer header',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    
    logger.warn('CSRF check failed: invalid origin', {
      ip: clientIP,
      origin: req.get('Origin'),
      referer: req.get('Referer'),
      host: req.get('Host'),
    });
    
    res.status(403).json({ error: 'CSRF validation failed: invalid origin' });
    return;
  }
  
  // 2. Double-submit cookie validation
  const cookieToken = req.cookies?.[CSRF_COOKIE_NAME];
  const headerToken = req.get(CSRF_HEADER_NAME);
  
  // Both must be present
  if (!cookieToken || !headerToken) {
    logAudit({
      actor: req.operator?.email || 'unknown',
      action: 'csrf_check',
      result: 'denied',
      reason: 'Missing CSRF token',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    
    logger.warn('CSRF check failed: missing token', {
      ip: clientIP,
      hasCookie: !!cookieToken,
      hasHeader: !!headerToken,
    });
    
    res.status(403).json({ error: 'CSRF validation failed: missing token' });
    return;
  }
  
  // Tokens must match (constant-time comparison to prevent timing attacks)
  if (!crypto.timingSafeEqual(Buffer.from(cookieToken), Buffer.from(headerToken))) {
    logAudit({
      actor: req.operator?.email || 'unknown',
      action: 'csrf_check',
      result: 'denied',
      reason: 'CSRF token mismatch',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    
    logger.warn('CSRF check failed: token mismatch', { ip: clientIP });
    
    res.status(403).json({ error: 'CSRF validation failed: token mismatch' });
    return;
  }
  
  next();
}

/**
 * Middleware to ensure CSRF cookie exists
 * Apply to all routes to ensure a token is always available
 */
export function ensureCsrfCookie(req: Request, res: Response, next: NextFunction): void {
  if (!req.cookies?.[CSRF_COOKIE_NAME]) {
    const token = generateCsrfToken();
    setCsrfCookie(res, token);
  }
  next();
}
