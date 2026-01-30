import { Request, Response, NextFunction } from 'express';
import jwt from 'jsonwebtoken';
import { AppError } from './errorHandler';
import logger from '../utils/logger';
import { isAuthTokenPayload } from '../types/auth';
import type { AuthTokenPayload } from '../types/auth';
import { isTokenBlacklisted } from '../services/jwtBlacklist.service';

// Extend Express Request to include user
declare module 'express-serve-static-core' {
  interface Request {
    user?: AuthTokenPayload;
  }
}

/**
 * Authentication middleware - verifies JWT token and attaches user to request
 * 
 * Security features:
 * - JWT signature verification
 * - Token expiration validation
 * - Blacklist checking for invalidated tokens (logout)
 */
export const authenticate = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    // In test harnesses we sometimes inject a synthetic user before hitting auth middleware.
    // Honor that short-circuit to keep controller/unit tests lightweight while preserving prod behavior.
    if (process.env.NODE_ENV === 'test' && req.user) {
      next();
      return;
    }
    // Get token from Authorization header or httpOnly cookie (access_token)
    const authHeader = req.headers.authorization;
    let token: string | null = null;
    if (authHeader && authHeader.startsWith('Bearer ')) {
      token = authHeader.substring(7);
    } else {
      const cookieName = process.env.ACCESS_TOKEN_COOKIE_NAME || 'access_token';
      const cookies = (req as Request & { cookies?: Record<string, string | undefined> }).cookies;
      token = (cookies && cookies[cookieName]) ? String(cookies[cookieName]) : null;
    }

    if (!token) {
      throw new AppError('No token provided', 401);
    }

    // Verify token
    const jwtSecret = process.env.JWT_SECRET;
    if (!jwtSecret) {
      logger.error('JWT_SECRET not configured');
      throw new AppError('Authentication configuration error', 500);
    }

    const decoded = jwt.verify(token, jwtSecret);

    if (!isAuthTokenPayload(decoded)) {
      throw new AppError('Invalid token payload', 401);
    }

    // Check if token has been blacklisted (e.g., after logout)
    // JTI (JWT ID) is stored in the 'jti' claim if present
    const jti = (decoded as jwt.JwtPayload).jti;
    if (jti) {
      const blacklisted = await isTokenBlacklisted(jti);
      if (blacklisted) {
        logger.info('Blacklisted token rejected', { jti, userId: decoded.userId });
        throw new AppError('Token has been invalidated', 401);
      }
    }

    // Attach user to request
    req.user = decoded;

    next();
  } catch (error) {
    if (error instanceof jwt.JsonWebTokenError) {
      next(new AppError('Invalid token', 401));
    } else if (error instanceof jwt.TokenExpiredError) {
      next(new AppError('Token expired', 401));
    } else {
      next(error);
    }
  }
};

/**
 * RBAC authorization middleware
 */
export const authorize = (roles: Array<'admin' | 'publisher' | 'readonly'>) => {
  return (req: Request, _res: Response, next: NextFunction) => {
    const role = req.user?.role ?? 'publisher';
    if (!roles.includes(role)) {
      return next(new AppError('Forbidden', 403));
    }
    next();
  };
};

/**
 * OPERATOR-ONLY authorization middleware
 * 
 * This middleware restricts access to system operators only (not customer admins).
 * Operators are identified by email whitelist via OPERATOR_EMAILS environment variable.
 * 
 * ╔════════════════════════════════════════════════════════════════════════════╗
 * ║ CRITICAL: Customer 'admin' role does NOT equal operator access             ║
 * ╠════════════════════════════════════════════════════════════════════════════╣
 * ║                                                                            ║
 * ║ CUSTOMER ROLES (authorize middleware):                                     ║
 * ║   - admin: Organization admin - billing, team, settings                    ║
 * ║   - publisher: Standard user - placements, analytics, tools                ║
 * ║   - readonly: View-only access                                             ║
 * ║                                                                            ║
 * ║ OPERATOR ACCESS (this middleware):                                         ║
 * ║   - Requires email in OPERATOR_EMAILS whitelist                            ║
 * ║   - Completely separate from customer RBAC system                          ║
 * ║   - Controls system-wide infrastructure, reconciliation, admin panels      ║
 * ║                                                                            ║
 * ╚════════════════════════════════════════════════════════════════════════════╝
 */
export const operatorOnly = () => {
  return (req: Request, _res: Response, next: NextFunction) => {
    const userEmail = req.user?.email?.toLowerCase();
    
    // Get operator emails from environment
    const operatorEmails = (process.env.OPERATOR_EMAILS || '')
      .split(',')
      .map(email => email.trim().toLowerCase())
      .filter(Boolean);
    
    // Fail-secure: if no operator emails configured, deny all access
    if (operatorEmails.length === 0) {
      logger.warn('OPERATOR_EMAILS not configured - denying admin access', { email: userEmail });
      return next(new AppError('Operator access not configured', 403));
    }
    
    // Check if user's email is in the operator whitelist
    if (!userEmail || !operatorEmails.includes(userEmail)) {
      logger.warn('Non-operator attempted admin access', { 
        email: userEmail, 
        ip: req.ip,
        path: req.path 
      });
      return next(new AppError('Operator access required', 403));
    }
    
    logger.info('Operator access granted', { email: userEmail, path: req.path });
    next();
  };
};
