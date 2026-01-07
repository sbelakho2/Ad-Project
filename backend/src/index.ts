// Load environment variables first, before any other imports
import dotenv from 'dotenv';
import path from 'path';
dotenv.config({ path: path.join(__dirname, '../../../.env') });

import express, { Application, Request, Response, NextFunction } from 'express';
import cors from 'cors';
import cookieParser from 'cookie-parser';
import helmet from 'helmet';
import compression from 'compression';
import rateLimit from 'express-rate-limit';
import { logger } from './utils/logger';
import { errorHandler } from './middleware/errorHandler';
import { requestContextMiddleware } from './middleware/requestContext';
import apiRoutes from './routes';
import { initializeDatabase } from './database';
import { redis } from './utils/redis';
import { initializeQueues, shutdownQueues } from './queues/queueInitializer';
import { promRegister, httpRequestDurationSeconds, httpRequestsTotal } from './utils/prometheus';
import { authTieredRateLimiter } from './middleware/tieredRateLimiter';
import swaggerUi from 'swagger-ui-express';
import { getOpenAPIDocument } from './utils/openapi';
import csrfProtection from './middleware/csrf';
import metricsRouteMiddleware from './middleware/metricsRouteMiddleware';
import { createHealthCheckRoutes } from './routes/healthCheck';
import pool from './utils/postgres';
import { initVraConnectionRoutes } from './routes/vraConnections.routes';
import { issueCsrfToken } from './middleware/csrf';
import { SchemaGate } from './utils/schemaGate';
import { initAnalyticsRollupScheduler } from './services/analyticsRollupScheduler';
import { initMigrationGuardrailScheduler } from './services/migrationGuardrailScheduler';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';

// Validate environment variables (fails fast with helpful errors)
import { env } from './config/env';

const app: Application = express();
const PORT = env.PORT;
const API_VERSION = env.API_VERSION;

// Phase 1 (Infra readiness): non-breaking production posture warnings
// These logs help operators ensure FRA1/TLS-aligned settings without changing business logic.
(() => {
  try {
    if (process.env.NODE_ENV === 'production') {
      // 1) DATABASE_URL should enforce TLS
      const dbUrl = process.env.DATABASE_URL || '';
      if (dbUrl && !/sslmode=require/i.test(dbUrl)) {
        logger.warn(
          'DATABASE_URL does not include sslmode=require — add ?sslmode=require for managed Postgres in production.'
        );
      }

      // 2) REDIS_URL should include a password and target private network host
      const redisUrl = process.env.REDIS_URL || '';
      if (redisUrl) {
        try {
          const u = new URL(redisUrl);
          const hasPassword = (u.password && u.password.length > 0);
          if (!hasPassword) {
            logger.warn('REDIS_URL has no password — set a strong password and require AUTH in production.');
          }
          if (u.hostname && !['redis', 'localhost'].includes(u.hostname) && /^[0-9.]+$/.test(u.hostname)) {
            logger.warn('REDIS_URL points to a raw IP — ensure Redis is not publicly exposed and is reachable only on private network.');
          }
        } catch {
          logger.warn('REDIS_URL is not a valid URL — expected e.g. redis://:<password>@redis:6379/0');
        }
      }

      // 3) CORS allowlist should include console + apex domains
      const corsAllow = (process.env.CORS_ALLOWLIST || process.env.CORS_ORIGIN || '').split(',').map(s => s.trim());
      const requiredOrigins = ['https://console.apexmediation.ee', 'https://apexmediation.ee'];
      for (const origin of requiredOrigins) {
        if (!corsAllow.includes(origin)) {
          logger.warn(`CORS allowlist does not include ${origin} — add it for production.`);
        }
      }
    }
  } catch {
    // best-effort warnings only; never block startup
  }
})();

// Configure trust proxy if behind reverse proxy (load balancer, nginx, etc.)
if (env.TRUST_PROXY) {
  app.set('trust proxy', 1);
  logger.info('Trust proxy enabled - respecting X-Forwarded-* headers');
}

// Security middleware
app.use(helmet({
  crossOriginOpenerPolicy: { policy: 'same-origin' },
  contentSecurityPolicy: process.env.ENABLE_CSP === '1' ? undefined : false,
}));

// Strict CORS allowlist
const splitCsv = (value: string | undefined): string[] =>
  (value || '')
    .split(',')
    .map(s => s.trim())
    .filter(Boolean);

const uniq = <T,>(values: T[]): T[] => Array.from(new Set(values));

const DEFAULT_LOCAL_ORIGINS = [
  'http://localhost:3000',
  'http://127.0.0.1:3000',
  'http://localhost:3001',
  'http://127.0.0.1:3001',
];

const DEFAULT_PROD_ORIGINS = [
  'https://console.apexmediation.ee',
  'https://apexmediation.ee',
  'https://www.apexmediation.ee',
];

const allowlist = uniq([
  ...splitCsv(env.CORS_ALLOWLIST),
  ...splitCsv(env.CORS_ORIGIN),
  ...(env.NODE_ENV === 'production' ? DEFAULT_PROD_ORIGINS : DEFAULT_LOCAL_ORIGINS),
]);

const corsOptions: cors.CorsOptions = {
  origin: (origin, callback) => {
    if (!origin) return callback(null, true); // allow non-browser clients
    if (allowlist.includes(origin)) return callback(null, true);

    // Reject without throwing (avoid turning CORS misses into 500s)
    logger.warn('CORS origin rejected', { origin });
    return callback(null, false);
  },
  credentials: true,
  methods: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'],
  allowedHeaders: ['Content-Type', 'Authorization', 'X-Publisher-Id', 'X-CSRF-Token'],
};

app.use(cors(corsOptions));
// Ensure preflight requests always get a consistent 204 response when allowed.
app.options('*', cors(corsOptions));

// Rate limiting - Tiered by customer plan
// Industry benchmarks: Stripe 100/sec, GitHub 5000/hour, Twitter Pro 900/15min
// Default: 1000 req/min (Starter tier) - much higher than previous 100/15min
import { defaultRateLimiter, isRateLimitExempt } from './middleware/tieredRateLimiter';
import { TIER_RATE_LIMITS } from './config/rateLimits';

// Log rate limit configuration at startup
logger.info('Rate limiting configured', {
  starterGlobal: `${TIER_RATE_LIMITS.starter.global.max} req/${TIER_RATE_LIMITS.starter.global.windowMs / 1000}s`,
  growthGlobal: `${TIER_RATE_LIMITS.growth.global.max} req/${TIER_RATE_LIMITS.growth.global.windowMs / 1000}s`,
  scaleGlobal: `${TIER_RATE_LIMITS.scale.global.max} req/${TIER_RATE_LIMITS.scale.global.windowMs / 1000}s`,
  enterpriseGlobal: `${TIER_RATE_LIMITS.enterprise.global.max} req/${TIER_RATE_LIMITS.enterprise.global.windowMs / 1000}s`,
});

const limiter = rateLimit({
  windowMs: parseInt(process.env.RATE_LIMIT_WINDOW_MS || '60000'), // 1 minute default (was 15 minutes)
  max: parseInt(process.env.RATE_LIMIT_MAX_REQUESTS || '1000'),    // 1000/min default (was 100/15min = 6.7/min)
  standardHeaders: true,
  legacyHeaders: false,
  message: {
    success: false,
    error: 'Too many requests',
    message: 'Rate limit exceeded. Upgrade your plan for higher limits.',
    upgradeUrl: '/pricing',
  },
  skip: (req: Request) => {
    const originalUrl = req.originalUrl || '';

    // Auth routes have their own stricter limiter
    if (originalUrl.startsWith(`/api/${API_VERSION}/auth`)) return true;

    // Public status/SLO endpoints must be resilient for customer-facing pages
    // These are cache-protected and need high availability
    if (req.method === 'GET' || req.method === 'HEAD') {
      if (originalUrl.startsWith(`/api/${API_VERSION}/status.json`)) return true;
      if (originalUrl.startsWith(`/api/${API_VERSION}/slo.json`)) return true;
    }

    // High-volume SDK ingest endpoints have their own analytics limiter
    if (originalUrl.startsWith(`/api/${API_VERSION}/analytics/events/`)) return true;

    // Health/readiness endpoints are always exempt
    if (isRateLimitExempt(originalUrl)) return true;

    return false;
  },
});
app.use(`/api/${API_VERSION}/`, limiter);

// Raw body parsing for Stripe webhooks (must be BEFORE express.json())
// Stripe needs the raw body buffer for signature verification
app.use(`/api/${API_VERSION}/webhooks/stripe`, express.raw({ type: 'application/json' }));

// Body parsing middleware
app.use(express.json({ limit: '10mb' }));
app.use(express.urlencoded({ extended: true, limit: '10mb' }));
app.use(compression());
app.use(cookieParser());

// Request context middleware (must be early to capture all logs)
app.use(requestContextMiddleware);
// Record RED metrics with low‑cardinality route_id labels
app.use(metricsRouteMiddleware);

// CSRF protection (double-submit cookie). Safe methods are ignored by csurf.
// Must be after cookie parser and before routes.
app.use(csrfProtection);

// Compatibility: some clients request CSRF at /auth/csrf (without /api/v1)
app.get('/auth/csrf', issueCsrfToken);

// RED metrics (Rate, Errors, Duration) by route
app.use((req: Request, res: Response, next: NextFunction) => {
  const end = httpRequestDurationSeconds.startTimer();
  res.on('finish', () => {
    try {
      const labels = {
        method: req.method,
        route: req.route?.path || req.path,
        status_code: String(res.statusCode)
      };
      
      // Record duration histogram (for p50, p95, p99)
      end(labels);
      
      // Record request counter (for rate)
      httpRequestsTotal.inc(labels);
    } catch {
      // noop
    }
  });
  next();
});

// Kubernetes-compatible health/readiness routes
app.use(createHealthCheckRoutes(pool));

app.get('/metrics', async (_req: Request, res: Response) => {
  try {
    res.set('Content-Type', promRegister.contentType);
    res.send(await promRegister.metrics());
  } catch (error) {
    logger.error('Failed to generate Prometheus metrics', {
      error: error instanceof Error ? error.message : 'Unknown error',
    });
    res.status(500).send('Unable to collect metrics');
  }
});

// API routes
// Attach stricter rate limiting for auth endpoints (tier-aware)
// Exclude session check endpoints (/me, /csrf) which are high-frequency and not security-sensitive
app.use(`/api/${API_VERSION}/auth`, (req, res, next) => {
  const path = req.path || '';
  // Session checks and CSRF tokens can be called frequently from the frontend
  // They should not be subject to the strict auth rate limiter
  if (path === '/me' || path === '/csrf') {
    return next();
  }
  return authTieredRateLimiter(req, res, next);
});

// OpenAPI/Swagger endpoints
app.get('/openapi.json', (_req, res) => {
  res.json(getOpenAPIDocument());
});
app.use('/docs', swaggerUi.serve, swaggerUi.setup(undefined, { swaggerUrl: '/openapi.json' }));

app.use(`/api/${API_VERSION}`, apiRoutes);

// 404 handler
app.use((req: Request, res: Response) => {
  res.status(404).json({
    success: false,
    error: 'Route not found',
    path: req.path,
  });
});

// Error handling middleware
app.use(errorHandler);

const startServer = async (): Promise<void> => {
  try {
    // Initialize PostgreSQL
    await initializeDatabase();

    // Validate schema requirements (fail fast on missing tables/extensions)
    logger.info('Validating database schema...');
    await SchemaGate.validateOrExit();
    logger.info('Schema validation passed');

    const runNodeScript = async (scriptPath: string, timeoutMs: number): Promise<void> => new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [scriptPath], {
        stdio: 'inherit',
        env: process.env,
      });

      const timer = setTimeout(() => {
        try {
          child.kill('SIGKILL');
        } catch {
          // noop
        }
        reject(new Error(`Timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      child.on('error', (err) => {
        clearTimeout(timer);
        reject(err);
      });

      child.on('exit', (code) => {
        clearTimeout(timer);
        if (code === 0) return resolve();
        reject(new Error(`Exited with code ${code ?? 'unknown'}`));
      });
    });

    // Zero-manual rollup rollout: if rollups are enabled, ensure the rollup tables exist.
    // Best-effort: log failures and continue (reporting/export already has safe fallbacks).
    if (env.ANALYTICS_ROLLUP_ENABLED) {
      const ensureScriptPath = path.join(__dirname, '../scripts/ensureRollupTables.js');
      try {
        await fs.access(ensureScriptPath);
        logger.info('Analytics rollups: ensuring rollup tables exist');
        await runNodeScript(ensureScriptPath, 2 * 60_000);
      } catch (error) {
        logger.warn('Analytics rollups: could not ensure rollup tables (continuing)', {
          scriptPath: ensureScriptPath,
          error: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    }

    // Analytics rollups (storage optimization + faster reporting)
    initAnalyticsRollupScheduler({
      enabled: env.ANALYTICS_ROLLUP_ENABLED,
      cronSpec: env.ANALYTICS_ROLLUP_CRON,
      lookbackDays: env.ANALYTICS_ROLLUP_LOOKBACK_DAYS,
    });

    // Migration guardrail scheduler (snapshot aggregation + automated evaluation)
    initMigrationGuardrailScheduler({
      enabled: env.MIGRATION_GUARDRAIL_ENABLED ?? true,
      snapshotCronSpec: env.MIGRATION_GUARDRAIL_SNAPSHOT_CRON,
      evaluationCronSpec: env.MIGRATION_GUARDRAIL_EVALUATION_CRON,
      pool,
    });

    // Initialize VRA services
    initVraConnectionRoutes(pool);
    logger.info('VRA connection services initialized');

    // Initialize Redis (non-blocking - log warning if fails)
    try {
      await redis.connect();
      logger.info('Redis cache enabled');
    } catch (error) {
      logger.warn('Redis connection failed - caching disabled', {
        error: error instanceof Error ? error.message : 'Unknown error',
      });
    }

    // Initialize job queues (requires Redis)
    if (redis.isReady()) {
      try {
        await initializeQueues();
        logger.info('Job queues enabled');
      } catch (error) {
        logger.warn('Job queue initialization failed - background jobs disabled', {
          error: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    } else {
      logger.warn('Redis not available - background jobs disabled');
    }

    const server = app.listen(PORT, () => {
      logger.info(`ApexMediation Backend API running on port ${PORT}`);
      logger.info(`Environment: ${process.env.NODE_ENV}`);
      logger.info(`API Version: ${API_VERSION}`);
    });

    // Store server reference for graceful shutdown
    (global as any).__httpServer = server;
  } catch (error) {
    logger.error('Failed to start server', {
      error: error instanceof Error ? error.message : 'Unknown error',
      stack: error instanceof Error ? error.stack : undefined,
    });
    process.exit(1);
  }
};

// ========================================
// Process Crash Handlers
// ========================================

process.on('unhandledRejection', (reason: unknown, promise: Promise<unknown>) => {
  logger.error('Unhandled Promise Rejection', {
    reason: reason instanceof Error ? { message: reason.message, stack: reason.stack } : reason,
    promise: String(promise),
  });
  // In production, you may want to exit after logging
  if (process.env.NODE_ENV === 'production') {
    logger.error('Exiting due to unhandled rejection in production');
    process.exit(1);
  }
});

process.on('uncaughtException', (error: Error, origin: string) => {
  logger.error('Uncaught Exception', {
    message: error.message,
    stack: error.stack,
    origin,
  });
  // Always exit on uncaught exceptions - the process is in an undefined state
  process.exit(1);
});

// ========================================
// Graceful Shutdown
// ========================================

const gracefulShutdown = async (signal: string): Promise<void> => {
  logger.info(`${signal} received, shutting down gracefully...`);

  const shutdownTimeout = setTimeout(() => {
    logger.error('Graceful shutdown timed out, forcing exit');
    process.exit(1);
  }, 30000); // 30 second timeout

  try {
    // 1. Stop accepting new connections
    const server = (global as any).__httpServer;
    if (server) {
      await new Promise<void>((resolve, reject) => {
        server.close((err?: Error) => {
          if (err) reject(err);
          else resolve();
        });
      });
      logger.info('HTTP server closed');
    }

    // 2. Shutdown job queues (waits for in-progress jobs)
    await shutdownQueues();
    logger.info('Job queues shut down');

    // 3. Disconnect Redis
    await redis.disconnect();
    logger.info('Redis disconnected');

    clearTimeout(shutdownTimeout);
    logger.info('Graceful shutdown completed');
    process.exit(0);
  } catch (error) {
    clearTimeout(shutdownTimeout);
    logger.error('Error during graceful shutdown', { error });
    process.exit(1);
  }
};

process.on('SIGTERM', () => void gracefulShutdown('SIGTERM'));
process.on('SIGINT', () => void gracefulShutdown('SIGINT'));

void startServer();

export default app;
