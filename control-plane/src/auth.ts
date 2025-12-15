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
function getClientIP(req: Request): string {
  const forwarded = req.get('X-Forwarded-For');
  if (forwarded) {
    return forwarded.split(',')[0].trim();
  }
  return req.ip || req.socket?.remoteAddress || 'unknown';
}
