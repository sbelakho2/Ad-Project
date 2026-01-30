/**
 * OPERATOR AUTHENTICATION
 * 
 * COMPLETELY SEPARATE from customer auth:
 * - Separate JWT secret
 * - Separate token format
 * - Separate identity store (env-based operator credentials)
 * - No shared tokens with customer API
 */

import { Request, Response, NextFunction } from 'express';
import jwt from 'jsonwebtoken';
import bcrypt from 'bcryptjs';
import { getConfig, getOperatorCredentials } from './config';
import { logger, logAudit } from './logger';

// ============================================
// IP ALLOWLIST & RATE LIMITING
// ============================================

// In-memory rate limit store (keyed by IP)
const loginAttempts = new Map<string, { count: number; firstAttempt: number }>();

// Rate limit config
const RATE_LIMIT_WINDOW_MS = 15 * 60 * 1000; // 15 minutes
const RATE_LIMIT_MAX_ATTEMPTS = 5; // 5 attempts per window

/**
 * Check if IP is within a CIDR range
 * Supports IPv4 only (IPv6 not required for this use case)
 * 
 * Note: Uses unsigned right shift (>>>) to ensure 32-bit operations
 * treat the result as unsigned, avoiding issues with high first octets
 * like 192.168.x.x where 192 << 24 would otherwise be negative.
 */
function ipInCidr(ip: string, cidr: string): boolean {
  const [range, bitsStr] = cidr.split('/');
  const bits = bitsStr ? parseInt(bitsStr, 10) : 32;
  
  if (bits < 0 || bits > 32) return false;
  
  const ipParts = ip.split('.').map(Number);
  const rangeParts = range.split('.').map(Number);
  
  if (ipParts.length !== 4 || rangeParts.length !== 4) return false;
  if (ipParts.some(p => isNaN(p) || p < 0 || p > 255)) return false;
  if (rangeParts.some(p => isNaN(p) || p < 0 || p > 255)) return false;
  
  // Use >>> 0 to convert to unsigned 32-bit integer
  // This prevents issues with high first octets (e.g., 192 << 24 = -1073741824 signed)
  const ipNum = ((ipParts[0] << 24) | (ipParts[1] << 16) | (ipParts[2] << 8) | ipParts[3]) >>> 0;
  const rangeNum = ((rangeParts[0] << 24) | (rangeParts[1] << 16) | (rangeParts[2] << 8) | rangeParts[3]) >>> 0;
  const mask = bits === 0 ? 0 : (~0 << (32 - bits)) >>> 0;
  
  return (ipNum & mask) === (rangeNum & mask);
}

/**
 * Check if IP is allowed
 * Supports single IPs and CIDR notation
 */
function isIpAllowed(ip: string, allowlist: string[]): boolean {
  if (allowlist.length === 0) return true; // Empty allowlist = allow all
  
  // Normalize IPv6-mapped IPv4 addresses
  let normalizedIp = ip;
  if (ip.startsWith('::ffff:')) {
    normalizedIp = ip.substring(7);
  }
  
  for (const entry of allowlist) {
    const trimmed = entry.trim();
    if (!trimmed) continue;
    
    if (trimmed.includes('/')) {
      // CIDR notation
      if (ipInCidr(normalizedIp, trimmed)) return true;
    } else {
      // Single IP
      if (normalizedIp === trimmed) return true;
    }
  }
  
  return false;
}

/**
 * IP Allowlist Middleware
 * Blocks requests from IPs not in OPERATOR_IP_ALLOWLIST
 */
export function requireAllowedIP(req: Request, res: Response, next: NextFunction): void {
  const clientIP = getClientIP(req);
  const allowlistEnv = process.env.OPERATOR_IP_ALLOWLIST;
  
  // If no allowlist configured, allow all (but log warning in production)
  if (!allowlistEnv || allowlistEnv.trim() === '') {
    if (process.env.NODE_ENV === 'production') {
      logger.warn('OPERATOR_IP_ALLOWLIST not configured - allowing all IPs');
    }
    return next();
  }
  
  const allowlist = allowlistEnv.split(',').map(s => s.trim()).filter(s => s.length > 0);
  
  if (!isIpAllowed(clientIP, allowlist)) {
    logAudit({
      actor: 'unknown',
      action: 'ip_blocked',
      result: 'denied',
      reason: 'IP not in allowlist',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    
    logger.warn('IP blocked by allowlist', { ip: clientIP });
    res.status(403).json({ error: 'Access denied' });
    return;
  }
  
  next();
}

/**
 * Rate Limiting Middleware for Login
 * Prevents brute-force attacks by limiting login attempts per IP
 */
export function rateLimitLogin(req: Request, res: Response, next: NextFunction): void {
  const clientIP = getClientIP(req);
  const now = Date.now();
  
  // Clean up old entries
  for (const [ip, data] of loginAttempts.entries()) {
    if (now - data.firstAttempt > RATE_LIMIT_WINDOW_MS) {
      loginAttempts.delete(ip);
    }
  }
  
  const attempts = loginAttempts.get(clientIP);
  
  if (attempts) {
    if (now - attempts.firstAttempt > RATE_LIMIT_WINDOW_MS) {
      // Window expired, reset
      loginAttempts.delete(clientIP);
    } else if (attempts.count >= RATE_LIMIT_MAX_ATTEMPTS) {
      const remainingMs = RATE_LIMIT_WINDOW_MS - (now - attempts.firstAttempt);
      const remainingMinutes = Math.ceil(remainingMs / 60000);
      
      logAudit({
        actor: 'unknown',
        action: 'rate_limited',
        result: 'denied',
        reason: `Too many login attempts (${RATE_LIMIT_MAX_ATTEMPTS} in ${RATE_LIMIT_WINDOW_MS / 60000} min)`,
        ip: clientIP,
        userAgent: req.get('User-Agent'),
      });
      
      logger.warn('Login rate limited', { ip: clientIP, attempts: attempts.count });
      res.status(429).json({ 
        error: 'Too many login attempts',
        retryAfterMinutes: remainingMinutes,
      });
      return;
    }
  }
  
  next();
}

/**
 * Record a login attempt (call after failed login)
 */
export function recordLoginAttempt(ip: string): void {
  const now = Date.now();
  const attempts = loginAttempts.get(ip);
  
  if (attempts) {
    attempts.count++;
  } else {
    loginAttempts.set(ip, { count: 1, firstAttempt: now });
  }
}

/**
 * Clear login attempts for an IP (call after successful login)
 */
export function clearLoginAttempts(ip: string): void {
  loginAttempts.delete(ip);
}

// ============================================
// JWT AUTHENTICATION
// ============================================

// Operator token payload - completely separate from customer tokens
interface OperatorToken {
  sub: string;        // operator email
  role: 'operator';   // always 'operator' - no customer roles here
  iat: number;
  exp: number;
}

declare global {
  namespace Express {
    interface Request {
      operator?: {
        email: string;
        role: 'operator';
      };
    }
  }
}

/**
 * Operator Authentication Middleware
 * Validates operator JWT tokens (separate from customer tokens)
 */
export function requireOperator(req: Request, res: Response, next: NextFunction): void {
  const authHeader = req.get('Authorization');
  const clientIP = getClientIP(req);
  
  if (!authHeader?.startsWith('Bearer ')) {
    logAudit({
      actor: 'unknown',
      action: 'auth_attempt',
      result: 'denied',
      reason: 'No bearer token',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    res.status(401).json({ error: 'Authentication required' });
    return;
  }
  
  const token = authHeader.substring(7);
  
  try {
    const cfg = getConfig();
    const decoded = jwt.verify(token, cfg.OPERATOR_JWT_SECRET) as OperatorToken;
    
    if (decoded.role !== 'operator') {
      throw new Error('Invalid token role');
    }
    
    req.operator = {
      email: decoded.sub,
      role: 'operator',
    };
    
    next();
  } catch (err) {
    logAudit({
      actor: 'unknown',
      action: 'auth_attempt',
      result: 'denied',
      reason: 'Invalid token',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    res.status(401).json({ error: 'Invalid or expired token' });
  }
}

/**
 * Authenticate operator with email/password
 * Validates username/password only (does not issue tokens)
 */
export async function verifyOperatorCredentials(
  email: string,
  password: string,
  ip: string,
  userAgent?: string
): Promise<{ email: string } | null> {
  const credentials = getOperatorCredentials();
  const normalizedEmail = email.toLowerCase();
  const storedHash = credentials.get(normalizedEmail);
  
  if (!storedHash) {
    logAudit({
      actor: email,
      action: 'login_attempt',
      result: 'denied',
      reason: 'Unknown operator email',
      ip,
      userAgent,
    });
    return null;
  }
  
  const isValid = await bcrypt.compare(password, storedHash);
  
  if (!isValid) {
    logAudit({
      actor: email,
      action: 'login_attempt',
      result: 'denied',
      reason: 'Invalid password',
      ip,
      userAgent,
    });
    return null;
  }

  return { email: normalizedEmail };
}

/**
 * Issue operator JWT token (call only after device is approved)
 */
export function issueOperatorToken(params: {
  email: string;
  ip: string;
  userAgent?: string;
}): { token: string; expiresIn: number } {
  const cfg = getConfig();
  const ttlHours = parseInt(cfg.OPERATOR_SESSION_TTL_HOURS, 10);
  const expiresIn = ttlHours * 60 * 60; // seconds

  const token = jwt.sign(
    {
      sub: params.email.toLowerCase(),
      role: 'operator',
    } as Omit<OperatorToken, 'iat' | 'exp'>,
    cfg.OPERATOR_JWT_SECRET,
    { expiresIn }
  );

  logAudit({
    actor: params.email,
    action: 'login_success',
    result: 'success',
    ip: params.ip,
    userAgent: params.userAgent,
  });

  logger.info('Operator authenticated', { email: params.email, ip: params.ip });

  return { token, expiresIn };
}

// Helper: Get client IP from request
export function getClientIP(req: Request): string {
  const forwarded = req.get('X-Forwarded-For');
  if (forwarded) {
    return forwarded.split(',')[0].trim();
  }
  return req.ip || req.socket?.remoteAddress || 'unknown';
}

// Export internal functions for testing
export const _internal = {
  ipInCidr,
  isIpAllowed,
};
