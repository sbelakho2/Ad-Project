import express, { Application, Request, Response, NextFunction } from 'express';
import cors from 'cors';
import helmet from 'helmet';
import compression from 'compression';
import rateLimit from 'express-rate-limit';
import cookieParser from 'cookie-parser';
import { logger } from '../../src/utils/logger';
import { promRegister } from '../../src/utils/prometheus';
import { errorHandler } from '../../src/middleware/errorHandler';
import { killSwitchGuard } from '../../src/middleware/featureFlags';
import apiRoutes from '../../src/routes';

/**
 * Create Express app for testing without starting a server
 * This avoids port conflicts when running multiple test suites
 */
export const createTestApp = (): Application => {
  const app: Application = express();

  // Security middleware
  app.use(helmet());
  app.use(cors({
    origin: process.env.CORS_ORIGIN || 'http://localhost:3000',
    credentials: true,
  }));

  // Rate limiting
  const limiter = rateLimit({
    windowMs: parseInt(process.env.RATE_LIMIT_WINDOW_MS || '900000'),
    max: parseInt(process.env.RATE_LIMIT_MAX_REQUESTS || '100'),
    message: 'Too many requests from this IP, please try again later.',
  });
  app.use('/api/v1/', limiter);

  // Body parsing middleware
  app.use(express.json({ limit: '10mb' }));
  app.use(express.urlencoded({ extended: true, limit: '10mb' }));
  app.use(compression());
  app.use(cookieParser());

  // Request logging
  app.use((req: Request, res: Response, next: NextFunction) => {
    logger.info(`${req.method} ${req.path}`, {
      ip: req.ip,
    });
    next();
  });

  // Health check endpoint
  app.get('/health', (req: Request, res: Response) => {
    res.json({
      status: 'healthy',
      timestamp: new Date().toISOString(),
      uptime: process.uptime(),
      environment: process.env.NODE_ENV,
    });
  });

  // Metrics endpoint (mirrors main server), required by observability tests
  app.get('/metrics', async (_req: Request, res: Response) => {
    try {
      res.set('Content-Type', promRegister.contentType);
      res.send(await promRegister.metrics());
    } catch (error) {
      res.status(500).send('Unable to collect metrics');
    }
  });

  // In test mode, provide a default authenticated user unless tests opt out with header 'noauth: 1'
  if (process.env.NODE_ENV === 'test') {
    app.use((req: Request & { user?: any }, _res: Response, next: NextFunction) => {
      if (!('noauth' in req.headers)) {
        req.user = { userId: 'user-1', email: 'user@example.com', publisherId: 'pub-1' };
      }
      next();
    });
  }

  // Mount kill-switch guard middleware before routes to mirror production behavior
  app.use(killSwitchGuard);
  
  // Skip CSRF in lightweight test app to keep tests simple and avoid token choreography
  // (CSRF is covered in integration/e2e layers)

  // API routes
  app.use('/api/v1', apiRoutes);

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

  return app;
};
