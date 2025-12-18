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
const allowlist = (process.env.CORS_ALLOWLIST || process.env.CORS_ORIGIN || 'http://localhost:3000')
  .split(',')
  .map(s => s.trim())
  .filter(Boolean);

app.use(cors({
  origin: (origin, callback) => {
    if (!origin) return callback(null, true); // allow non-browser clients
    if (allowlist.includes(origin)) return callback(null, true);
    return callback(new Error('CORS: origin not allowed'));
  },
  credentials: true,
  methods: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'],
  allowedHeaders: ['Content-Type', 'Authorization'],
}));

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
app.use(`/api/${API_VERSION}/auth`, authTieredRateLimiter);

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

    app.listen(PORT, () => {
      logger.info(`ApexMediation Backend API running on port ${PORT}`);
      logger.info(`Environment: ${process.env.NODE_ENV}`);
      logger.info(`API Version: ${API_VERSION}`);
    });
  } catch (error) {
    logger.error('Failed to start server', {
      error: error instanceof Error ? error.message : 'Unknown error',
      stack: error instanceof Error ? error.stack : undefined,
    });
    process.exit(1);
  }
};

// Graceful shutdown
process.on('SIGTERM', async () => {
  logger.info('SIGTERM received, shutting down gracefully...');
  
  try {
    await shutdownQueues();
    await redis.disconnect();
    process.exit(0);
  } catch (error) {
    logger.error('Error during shutdown', { error });
    process.exit(1);
  }
});

process.on('SIGINT', async () => {
  logger.info('SIGINT received, shutting down gracefully...');
  
  try {
    await shutdownQueues();
    await redis.disconnect();
    process.exit(0);
  } catch (error) {
    logger.error('Error during shutdown', { error });
    process.exit(1);
  }
});

void startServer();

export default app;
