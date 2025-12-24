import { Request, Response, NextFunction } from 'express';
import { randomUUID } from 'crypto';
import { asyncLocalStorage, logger } from '../utils/logger';

/**
 * Request ID middleware
 * - Generates or reuses x-request-id header
 * - Stores requestId in async local storage for correlation
 * - Adds requestId to response headers
 * - Extracts userId and tenantId from authenticated requests
 */
type ReqWithUser = Request & { user?: { id?: string; userId?: string; tenantId?: string } };

export function requestContextMiddleware(req: Request, res: Response, next: NextFunction) {
  // Generate or use existing request ID
  const requestId = (req.headers['x-request-id'] as string) || randomUUID();
  
  // Add request ID to response headers for tracing
  res.setHeader('x-request-id', requestId);
  
  // Extract user context from authenticated requests
  const r = req as ReqWithUser & { tenant?: { id?: string } };
  const userId = r.user?.id || r.user?.userId;
  const tenantId = r.user?.tenantId || r.tenant?.id;
  
  // Store context in async local storage
  const context = {
    requestId,
    ...(userId && { userId }),
    ...(tenantId && { tenantId }),
  };
  
  // Run the request within this context
  asyncLocalStorage.run(context, () => {
    // Log only errors and warnings (not every request)
    next();
  });
}

/**
 * Helper to get current request context
 * Useful for manual logging in controllers/services
 */
export function getRequestContext() {
  return asyncLocalStorage.getStore();
}

/**
 * Helper to create a child logger with additional context
 * Example: const log = createContextLogger({ operation: 'billing' });
 */
export function createContextLogger(additionalContext: Record<string, unknown>) {
  const context = asyncLocalStorage.getStore() || {};
  return logger.child({ ...context, ...additionalContext });
}
