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
import { getConfig, getOperatorCredentials, getAllowedIPs } from './config';
import { logger, logAudit } from './logger';

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
 * IP Allowlist Middleware
 * Rejects requests from non-allowlisted IPs before any other processing
 */
export function ipAllowlist(req: Request, res: Response, next: NextFunction): void {
  const allowedIPs = getAllowedIPs();
  const clientIP = getClientIP(req);
  
  // Check if IP is allowed
  const isAllowed = allowedIPs.some(cidr => isIPInCIDR(clientIP, cidr));
  
  if (!isAllowed) {
    logger.warn('IP not in allowlist', { ip: clientIP, allowedIPs });
    logAudit({
      actor: 'unknown',
      action: 'access_attempt',
      result: 'denied',
      reason: 'IP not in allowlist',
      ip: clientIP,
      userAgent: req.get('User-Agent'),
    });
    
    // Return generic 404 - don't reveal this is an admin endpoint
    res.status(404).json({ error: 'Not found' });
    return;
  }
  
  next();
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
 * Returns JWT token on success
 */
export async function authenticateOperator(
  email: string,
  password: string,
  ip: string,
  userAgent?: string
): Promise<{ token: string; expiresIn: number } | null> {
  const credentials = getOperatorCredentials();
  const storedHash = credentials.get(email.toLowerCase());
  
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
  
  const cfg = getConfig();
  const ttlHours = parseInt(cfg.OPERATOR_SESSION_TTL_HOURS, 10);
  const expiresIn = ttlHours * 60 * 60; // seconds
  
  const token = jwt.sign(
    {
      sub: email.toLowerCase(),
      role: 'operator',
    } as Omit<OperatorToken, 'iat' | 'exp'>,
    cfg.OPERATOR_JWT_SECRET,
    { expiresIn }
  );
  
  logAudit({
    actor: email,
    action: 'login_success',
    result: 'success',
    ip,
    userAgent,
  });
  
  logger.info('Operator authenticated', { email, ip });
  
  return { token, expiresIn };
}

// Helper: Get client IP from request
function getClientIP(req: Request): string {
  const forwarded = req.get('X-Forwarded-For');
  if (forwarded) {
    return forwarded.split(',')[0].trim();
  }
  return req.ip || req.socket?.remoteAddress || 'unknown';
}

// Helper: Check if IP is in CIDR range
function isIPInCIDR(ip: string, cidr: string): boolean {
  // Handle IPv6 localhost
  if (ip === '::1' && cidr === '::1/128') return true;
  if (ip === '::ffff:127.0.0.1' && cidr === '127.0.0.1/32') return true;
  
  // Simple CIDR check for IPv4
  const [network, bits] = cidr.split('/');
  const mask = bits ? parseInt(bits, 10) : 32;
  
  // Normalize IPv4-mapped IPv6
  const normalizedIP = ip.replace('::ffff:', '');
  const normalizedNetwork = network.replace('::ffff:', '');
  
  if (mask === 32) {
    return normalizedIP === normalizedNetwork;
  }
  
  // For ranges, do proper CIDR check
  const ipParts = normalizedIP.split('.').map(Number);
  const networkParts = normalizedNetwork.split('.').map(Number);
  
  if (ipParts.length !== 4 || networkParts.length !== 4) {
    return false;
  }
  
  const ipNum = (ipParts[0] << 24) | (ipParts[1] << 16) | (ipParts[2] << 8) | ipParts[3];
  const networkNum = (networkParts[0] << 24) | (networkParts[1] << 16) | (networkParts[2] << 8) | networkParts[3];
  const maskNum = ~((1 << (32 - mask)) - 1);
  
  return (ipNum & maskNum) === (networkNum & maskNum);
}
