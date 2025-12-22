import { Request, Response, NextFunction } from 'express';
import { logger } from '../utils/logger';
import { errorCounter } from '../utils/prometheus';

export class AppError extends Error {
  statusCode: number;
  isOperational: boolean;

  constructor(message: string, statusCode: number) {
    super(message);
    this.statusCode = statusCode;
    this.isOperational = true;

    Error.captureStackTrace(this, this.constructor);
  }
}

export const errorHandler = (
  err: Error | AppError,
  req: Request,
  res: Response,
  _next: NextFunction
) => {
  // Invalid JSON body (from express.json/body-parser)
  const parseErr = err as unknown as { status?: number; type?: string };
  if (
    (err instanceof SyntaxError && parseErr?.status === 400) ||
    parseErr?.type === 'entity.parse.failed'
  ) {
    try { errorCounter.inc({ type: 'operational' }); } catch (e) { void e; }
    logger.warn('Invalid JSON body', { path: req.path, method: req.method });
    return res.status(400).json({ success: false, error: 'Invalid JSON body' });
  }

  // CSRF errors (from csurf)
  const anyErr = err as unknown as { code?: string };
  if (anyErr && anyErr.code === 'EBADCSRFTOKEN') {
    try { errorCounter.inc({ type: 'operational' }); } catch (e) { void e; }
    logger.warn('CSRF token mismatch', { path: req.path, method: req.method });
    return res.status(403).json({ success: false, error: 'Invalid CSRF token' });
  }
  if (err instanceof AppError) {
    try { errorCounter.inc({ type: 'operational' }); } catch (e2) { void e2; }
    logger.error(`${err.statusCode} - ${err.message}`, {
      stack: err.stack,
      path: req.path,
      method: req.method,
    });

    return res.status(err.statusCode).json({
      success: false,
      error: err.message,
    });
  }

  // Unexpected errors
  try { errorCounter.inc({ type: 'unexpected' }); } catch (e3) { void e3; }
  logger.error('Unexpected error:', {
    error: err.message,
    stack: err.stack,
    path: req.path,
    method: req.method,
  });

  return res.status(500).json({
    success: false,
    error: 'An unexpected error occurred',
  });
};
