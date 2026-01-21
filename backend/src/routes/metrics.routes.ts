import { Router } from 'express';
import appApiKeyAuth from '../middleware/appApiKeyAuth';
import { authRateLimiter } from '../middleware/redisRateLimiter';
import { validate, sdkUsageMetricsSchema } from '../middleware/validation';
import * as metricsController from '../controllers/metrics.controller';

const router = Router();

/**
 * POST /api/v1/metrics/usage
 * Record SDK usage for billing and audit trails
 */
router.post('/usage', authRateLimiter, appApiKeyAuth, validate(sdkUsageMetricsSchema), metricsController.recordSdkUsage);

export default router;
