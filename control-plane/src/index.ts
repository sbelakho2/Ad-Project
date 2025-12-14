/**
 * CONTROL PLANE - MAIN ENTRY POINT
 * 
 * Separate control plane service for system administration.
 * Runs on its own port (4000) with its own hostname (admin.apexmediation.ee)
 * 
 * Security layers:
 * 1. IP allowlist - returns 404 for non-allowed IPs
 * 2. Operator JWT authentication - separate from customer auth
 * 3. Tiered action permissions - confirmation required for dangerous ops
 * 4. Full audit logging with 7-year retention
 */

import express, { Request, Response, NextFunction } from 'express';
import helmet from 'helmet';
import cors from 'cors';
import compression from 'compression';

import { loadConfig } from './config';
import { logger, logAudit } from './logger';
import { ipAllowlist } from './auth';
import { initializePools, closePools, getReadOnlyPool } from './database';
import routes from './routes';

// Load and validate configuration
const config = loadConfig();

const app = express();

// ============================================
// SECURITY MIDDLEWARE
// ============================================

// Helmet for security headers
app.use(helmet({
  contentSecurityPolicy: {
    directives: {
      defaultSrc: ["'self'"],
      scriptSrc: ["'self'"],
      styleSrc: ["'self'", "'unsafe-inline'"],
      imgSrc: ["'self'", 'data:'],
    },
  },
  hsts: {
    maxAge: 31536000,
    includeSubDomains: true,
    preload: true,
  },
}));

// CORS - only allow same origin (admin.* subdomain)
app.use(cors({
  origin: config.CORS_ORIGIN || false,
  credentials: true,
}));

// Trust proxy for accurate IP detection behind Caddy/nginx
app.set('trust proxy', true);

// IP Allowlist - FIRST security check
// Returns 404 for non-allowed IPs (stealth mode)
app.use(ipAllowlist);

// ============================================
// STANDARD MIDDLEWARE
// ============================================

// Compression
app.use(compression());

// JSON body parsing
app.use(express.json({ limit: '1mb' }));

// Request logging
app.use((req: Request, res: Response, next: NextFunction) => {
  const start = Date.now();
  const requestId = `req_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  
  // Attach request ID
  (req as any).requestId = requestId;
  
  res.on('finish', () => {
    const duration = Date.now() - start;
    
    // Don't log health checks at info level
    if (req.path === '/health' || req.path === '/ready') {
      logger.debug('Request completed', {
        requestId,
        method: req.method,
        path: req.path,
        status: res.statusCode,
        duration,
      });
    } else {
      logger.info('Request completed', {
        requestId,
        method: req.method,
        path: req.path,
        status: res.statusCode,
        duration,
        ip: req.ip,
        userAgent: req.get('User-Agent'),
      });
    }
  });
  
  next();
});

// ============================================
// ROUTES
// ============================================

app.use(routes);

// ============================================
// ERROR HANDLING
// ============================================

// 404 handler
app.use((req: Request, res: Response) => {
  res.status(404).json({ error: 'Not found' });
});

// Global error handler
app.use((err: Error, req: Request, res: Response, next: NextFunction) => {
  const requestId = (req as any).requestId || 'unknown';
  
  logger.error('Unhandled error', {
    requestId,
    error: err.message,
    stack: err.stack,
    path: req.path,
    method: req.method,
  });
  
  // Log audit for errors on action routes
  if (req.path.includes('/actions/')) {
    logAudit({
      actor: (req as any).operator?.username || 'unknown',
      action: `error:${req.path}`,
      result: 'failure',
      reason: err.message,
      ip: req.ip || 'unknown',
    });
  }
  
  // Don't leak error details in production
  if (config.NODE_ENV === 'production') {
    res.status(500).json({ error: 'Internal server error' });
  } else {
    res.status(500).json({ 
      error: err.message,
      stack: err.stack,
    });
  }
});

// ============================================
// SERVER STARTUP
// ============================================

async function startServer() {
  try {
    // Initialize database pools
    logger.info('Initializing database connections...');
    await initializePools();
    
    // Verify read-only connection
    const pool = getReadOnlyPool();
    const result = await pool.query('SELECT NOW() as time');
    logger.info('Database connection verified', { 
      serverTime: result.rows[0].time 
    });
    
    // Start HTTP server
    const server = app.listen(config.PORT, () => {
      logger.info(`Control plane started`, {
        port: config.PORT,
        nodeEnv: config.NODE_ENV,
        allowedIPs: config.ALLOWED_IPS.split(',').length,
      });
      
      console.log(`
╔══════════════════════════════════════════════════════════════╗
║                     CONTROL PLANE                            ║
║══════════════════════════════════════════════════════════════║
║  Status:    RUNNING                                          ║
║  Port:      ${config.PORT}                                          ║
║  Env:       ${config.NODE_ENV.padEnd(50)}║
║  Auth:      IP Allowlist + Operator JWT                      ║
║  DB Roles:  read-only, operator, break-glass                 ║
╚══════════════════════════════════════════════════════════════╝
      `);
    });
    
    // Graceful shutdown
    const shutdown = async (signal: string) => {
      logger.info(`Received ${signal}, shutting down gracefully...`);
      
      server.close(async () => {
        logger.info('HTTP server closed');
        
        await closePools();
        logger.info('Database pools closed');
        
        process.exit(0);
      });
      
      // Force exit after 30 seconds
      setTimeout(() => {
        logger.error('Forced shutdown after timeout');
        process.exit(1);
      }, 30000);
    };
    
    process.on('SIGTERM', () => shutdown('SIGTERM'));
    process.on('SIGINT', () => shutdown('SIGINT'));
    
    // Handle uncaught errors
    process.on('uncaughtException', (err) => {
      logger.error('Uncaught exception', { error: err.message, stack: err.stack });
      process.exit(1);
    });
    
    process.on('unhandledRejection', (reason, promise) => {
      logger.error('Unhandled rejection', { reason: String(reason) });
    });
    
  } catch (err) {
    logger.error('Failed to start server', { error: (err as Error).message });
    process.exit(1);
  }
}

startServer();
