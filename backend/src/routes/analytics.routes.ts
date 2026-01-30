import { Router } from 'express';
import { authenticate } from '../middleware/auth';
import { authenticateOrApiKey } from '../middleware/authEither';
import { appApiKeyAuth } from '../middleware/appApiKeyAuth';
import { cache, invalidatePublisherCache } from '../middleware/cache';
import { cacheTTL } from '../utils/redis';
import * as analyticsController from '../controllers/analytics.controller';
import { ingestByoSpans, getByoAdapterMetrics, getByoTraces } from '../controllers/byoTelemetry.controller';
import { authRateLimiter } from '../middleware/redisRateLimiter';

const router = Router();

// ========================================
// Dashboard/Reporting Endpoints (require auth)
// ========================================
// GET /api/v1/analytics/overview - Get analytics overview
router.get('/overview', authenticateOrApiKey, cache({ 
  ttl: cacheTTL.medium, // Cache for 5 minutes
  varyBy: ['query.startDate', 'query.endDate']
}), analyticsController.getOverview);

// GET /api/v1/analytics/timeseries - Get time series analytics data
router.get('/timeseries', authenticateOrApiKey, cache({ 
  ttl: cacheTTL.medium,
  varyBy: ['query.startDate', 'query.endDate', 'query.granularity']
}), analyticsController.getTimeSeries);

// GET /api/v1/analytics/impressions - Get impressions breakdown
router.get('/impressions', authenticateOrApiKey, cache({
  ttl: cacheTTL.medium,
  varyBy: ['query.startDate', 'query.endDate', 'query.groupBy']
}), analyticsController.getImpressions);

// GET /api/v1/analytics/performance - Get performance metrics
router.get('/performance', authenticateOrApiKey, cache({ 
  ttl: cacheTTL.long, // Cache for 30 minutes - more stable data
  varyBy: ['query.startDate', 'query.endDate', 'query.groupBy']
}), analyticsController.getPerformance);

// GET /api/v1/analytics/buffer-stats - Get event buffer stats (monitoring)
router.get('/buffer-stats', authenticateOrApiKey, cache({ 
  ttl: cacheTTL.short // Cache for 1 minute - near real-time
}), analyticsController.getBufferStats);

// ========================================
// Event Ingestion Endpoints (no auth for SDK)
// ========================================
// POST /api/v1/analytics/events/impressions - Record impression events
// SDK ingestion endpoints can be abused; apply rate limit and cache invalidation
router.post('/events/impressions', authRateLimiter, appApiKeyAuth, invalidatePublisherCache(), analyticsController.recordImpressions);

// POST /api/v1/analytics/events/clicks - Record click events
router.post('/events/clicks', authRateLimiter, appApiKeyAuth, invalidatePublisherCache(), analyticsController.recordClicks);

// POST /api/v1/analytics/events/revenue - Record revenue events
router.post('/events/revenue', authRateLimiter, appApiKeyAuth, invalidatePublisherCache(), analyticsController.recordRevenue);

// POST /api/v1/analytics/events/viewability - Record OMSDK/viewability events (SDK_CHECKS 7.3)
// Requires app API key auth for security - prevents anonymous event injection
router.post('/events/viewability', authRateLimiter, appApiKeyAuth, invalidatePublisherCache(), analyticsController.recordViewability);

// POST /api/v1/analytics/events/manifest-parse - Record SSAI manifest parse failures (best-effort)
// Requires app API key auth for security - prevents anonymous event injection
router.post('/events/manifest-parse', authRateLimiter, appApiKeyAuth, invalidatePublisherCache(), analyticsController.recordManifestParseErrors);

// GET /api/v1/analytics/viewability/:placementId - Get viewability summary for a placement
router.get('/viewability/:placementId', authenticateOrApiKey, cache({ 
  ttl: cacheTTL.medium,
  varyBy: ['query.startDate', 'query.endDate']
}), analyticsController.getViewabilitySummary);

// GET /api/v1/analytics/omsdk-status - Get OMSDK status for publisher
router.get('/omsdk-status', authenticateOrApiKey, cache({ 
  ttl: cacheTTL.medium 
}), analyticsController.getOmsdkStatus);

// GET /api/v1/analytics/sdk-versions - Get SDK version distribution
router.get('/sdk-versions', authenticateOrApiKey, cache({
  ttl: cacheTTL.medium,
  varyBy: ['query.startDate', 'query.endDate']
}), analyticsController.getSdkVersions);

export default router;

// ========================================
// BYO Observability (Adapter Spans) Endpoints
// ========================================
// POST /api/v1/analytics/byo/spans - Ingest sampled adapter span events from SDK (rate-limited)
router.post('/byo/spans', authRateLimiter, ingestByoSpans);

// GET /api/v1/analytics/byo/adapter-metrics - Summary metrics (auth required)
router.get(
  '/byo/adapter-metrics',
  authenticateOrApiKey,
  cache({ ttl: cacheTTL.short, varyBy: ['query.appId','query.placement','query.adapter','query.from','query.to'] }),
  getByoAdapterMetrics
);

// GET /api/v1/analytics/byo/traces - Recent sanitized traces (auth required)
router.get(
  '/byo/traces',
  authenticateOrApiKey,
  cache({ ttl: cacheTTL.short, varyBy: ['query.appId','query.placement','query.adapter','query.from','query.to','query.limit'] }),
  getByoTraces
);
