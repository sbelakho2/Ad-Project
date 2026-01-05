import { Request, Response, NextFunction } from 'express';
import { z } from 'zod';
import { AppError } from '../middleware/errorHandler';
import {
  getAnalyticsOverview,
  getAnalyticsTimeSeries,
  getImpressionsBreakdown,
  getPerformanceBreakdown,
} from '../services/analyticsPipeline';
import analyticsService from '../services/analyticsService';
import * as viewabilityService from '../services/viewabilityEvents';
import logger from '../utils/logger';
import config from '../config/index';
import { queueManager, QueueName } from '../queues/queueManager';
import { analyticsEventsEnqueuedTotal } from '../utils/prometheus';
import { recordPerformanceMetric } from '../services/performanceMetricsRecorder';

const parseQueryParam = (value: unknown): string | undefined => {
  if (typeof value === 'string') {
    return value;
  }

  if (Array.isArray(value)) {
    for (const item of value) {
      if (typeof item === 'string') {
        return item;
      }
    }
  }

  return undefined;
};

/**
 * Get analytics overview
 */
export const getOverview = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const overview = await getAnalyticsOverview(publisherId);

    res.json({
      success: true,
      data: overview,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get analytics time series data
 */
export const getTimeSeries = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const metricParam = parseQueryParam(req.query.metric);
    const metric = metricParam === 'impressions' || metricParam === 'clicks' ? metricParam : 'revenue';

    const startDateParam = parseQueryParam(req.query.startDate);
    const endDateParam = parseQueryParam(req.query.endDate);
    const granularityParam = parseQueryParam(req.query.granularity);
    const granularity = granularityParam === 'hour' ? 'hour' : 'day';

    const timeSeries = await getAnalyticsTimeSeries(
      publisherId,
      metric,
      startDateParam,
      endDateParam,
      granularity
    );

    res.json({
      success: true,
      data: timeSeries,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get performance metrics by placement/adapter
 */
export const getPerformance = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const groupByParam = parseQueryParam(req.query.groupBy);
    const groupBy = groupByParam === 'adapter' ? 'adapter' : 'placement';

    const startDateParam = parseQueryParam(req.query.startDate);
    const endDateParam = parseQueryParam(req.query.endDate);

    const performance = await getPerformanceBreakdown(publisherId, groupBy, startDateParam, endDateParam);

    res.json({
      success: true,
      data: performance,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get impressions breakdown
 */
export const getImpressions = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const startDateParam = parseQueryParam(req.query.startDate);
    const endDateParam = parseQueryParam(req.query.endDate);
    const groupByParam = parseQueryParam(req.query.groupBy);
    const groupBy = groupByParam === 'adapter' || groupByParam === 'placement' ? groupByParam : 'date';

    const data = await getImpressionsBreakdown(publisherId, groupBy, startDateParam, endDateParam);

    res.json({
      success: true,
      data,
    });
  } catch (error) {
    next(error);
  }
};

// =============================
// Ingestion helpers (producer)
// =============================

async function enqueueIfEnabled(kind: 'impressions' | 'clicks' | 'revenue', events: unknown[], publisherId?: string) {
  // Only enqueue when flag enabled and queue is ready; otherwise let callers fall back to direct service path
  try {
    if (!config.useRedisStreamsForAnalytics || !queueManager || !queueManager.isReady()) return false;
    const queue = (queueManager as any).getQueue ? (queueManager as any).getQueue(QueueName.ANALYTICS_INGEST) : null;
    if (!queue) return false;
    await queue.add(kind, { kind, events, publisherId }, {
      removeOnComplete: true,
      removeOnFail: { age: 3600 },
    });
    try { analyticsEventsEnqueuedTotal.inc({ kind }); } catch { /* noop */ }
    return true;
  } catch (e) {
    // On any error, skip enqueue so API path can attempt synchronous processing
    logger.warn('[Analytics] enqueue failed, falling back to direct insert', { error: (e as Error).message });
    return false;
  }
}
// ========================================
// Event Ingestion Endpoints
// ========================================

// Validation schemas for event ingestion
const impressionEventSchema = z.object({
  event_id: z.string().uuid(),
  timestamp: z.string().datetime(),
  publisher_id: z.string().uuid(),
  app_id: z.string().uuid(),
  placement_id: z.string().uuid(),
  adapter_id: z.string().uuid(),
  adapter_name: z.string(),
  ad_unit_id: z.string(),
  ad_format: z.enum(['banner', 'interstitial', 'rewarded', 'native', 'video']),
  country_code: z.string().length(2),
  device_type: z.enum(['phone', 'tablet', 'tv', 'desktop', 'console']),
  os: z.enum(['ios', 'android', 'web', 'tvos', 'macos', 'windows', 'linux']),
  os_version: z.string(),
  app_version: z.string(),
  sdk_version: z.string(),
  session_id: z.string().uuid(),
  user_id: z.string(),
  request_id: z.string().uuid(),
  bid_price_usd: z.number().nonnegative(),
  ecpm_usd: z.number().nonnegative(),
  latency_ms: z.number().int().nonnegative(),
  is_test_mode: z.boolean(),
});

const clickEventSchema = z.object({
  event_id: z.string().uuid(),
  timestamp: z.string().datetime(),
  impression_id: z.string().uuid(),
  publisher_id: z.string().uuid(),
  app_id: z.string().uuid(),
  placement_id: z.string().uuid(),
  adapter_id: z.string().uuid(),
  adapter_name: z.string(),
  click_url: z.string().url(),
  country_code: z.string().length(2),
  device_type: z.enum(['phone', 'tablet', 'tv', 'desktop', 'console']),
  os: z.enum(['ios', 'android', 'web', 'tvos', 'macos', 'windows', 'linux']),
  session_id: z.string().uuid(),
  user_id: z.string(),
  request_id: z.string().uuid(),
  time_to_click_ms: z.number().int().nonnegative(),
  is_verified: z.boolean(),
  is_test_mode: z.boolean(),
});

const revenueEventSchema = z.object({
  event_id: z.string().uuid(),
  timestamp: z.string().datetime(),
  publisher_id: z.string().uuid(),
  app_id: z.string().uuid(),
  placement_id: z.string().uuid(),
  adapter_id: z.string().uuid(),
  adapter_name: z.string(),
  impression_id: z.string().uuid(),
  revenue_type: z.enum(['impression', 'click', 'install', 'iap']),
  revenue_usd: z.number().nonnegative(),
  revenue_currency: z.string().length(3),
  revenue_original: z.number().nonnegative(),
  exchange_rate: z.number().positive(),
  ecpm_usd: z.number().nonnegative(),
  country_code: z.string().length(2),
  ad_format: z.enum(['banner', 'interstitial', 'rewarded', 'native', 'video']),
  os: z.enum(['ios', 'android', 'web', 'tvos', 'macos', 'windows', 'linux']),
  is_test_mode: z.boolean(),
  reconciliation_status: z.enum(['pending', 'matched', 'discrepancy']),
});

const manifestParseErrorSchema = z.object({
  timestamp: z.string().datetime().optional(),
  publisher_id: z.string().uuid().optional(),
  app_id: z.string().uuid().optional(),
  session_id: z.string().optional(),
  protocol: z.enum(['hls', 'dash']).optional(),
  sdk_platform: z.string().max(32).optional(),
  sdk_version: z.string().max(32).optional(),
  error_code: z.string().max(64).optional(),
  error_message: z.string().max(500).optional(),
  metadata: z.record(z.unknown()).optional(),
});

/**
 * POST /api/v1/analytics/events/impressions
 * Record impression events
 */
export const recordImpressions = async (
  req: Request,
  res: Response,
  _next?: NextFunction
): Promise<void> => {
  try {
    const { events } = req.body as { events?: unknown };

    if (!Array.isArray(events)) {
      res.status(400).json({
        success: false,
        error: 'events must be an array',
      });
      return;
    }

    // Validate all events
    const validatedEvents = events.map((event: unknown) => impressionEventSchema.parse(event));

    const sdk = (req as any).sdk as { publisherId: string; appId: string } | undefined;
    if (sdk) {
      for (const ev of validatedEvents) {
        if (ev.publisher_id !== sdk.publisherId || ev.app_id !== sdk.appId) {
          res.status(403).json({
            success: false,
            error: 'publisher_id/app_id do not match SDK key scope',
          });
          return;
        }
      }
    }

    // Record events
    await analyticsService.recordImpressions(validatedEvents);

    res.status(202).json({
      success: true,
      message: `Queued ${validatedEvents.length} impression events`,
      count: validatedEvents.length,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      res.status(400).json({
        success: false,
        error: 'Invalid event data',
        details: error.errors,
      });
      return;
    }

    logger.error('Failed to record impressions', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to record impression events',
    });
  }
};

/**
 * POST /api/v1/analytics/events/clicks
 * Record click events
 */
export const recordClicks = async (
  req: Request,
  res: Response,
  _next?: NextFunction
): Promise<void> => {
  try {
    const { events } = req.body as { events?: unknown };

    if (!Array.isArray(events)) {
      res.status(400).json({
        success: false,
        error: 'events must be an array',
      });
      return;
    }

    const validatedEvents = events.map((event: unknown) => clickEventSchema.parse(event));

    const sdk = (req as any).sdk as { publisherId: string; appId: string } | undefined;
    if (sdk) {
      for (const ev of validatedEvents) {
        if (ev.publisher_id !== sdk.publisherId || ev.app_id !== sdk.appId) {
          res.status(403).json({
            success: false,
            error: 'publisher_id/app_id do not match SDK key scope',
          });
          return;
        }
      }
    }

    await analyticsService.recordClicks(validatedEvents);

    res.status(202).json({
      success: true,
      message: `Queued ${validatedEvents.length} click events`,
      count: validatedEvents.length,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      res.status(400).json({
        success: false,
        error: 'Invalid event data',
        details: error.errors,
      });
      return;
    }

    logger.error('Failed to record clicks', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to record click events',
    });
  }
};

/**
 * POST /api/v1/analytics/events/revenue
 * Record revenue events
 */
export const recordRevenue = async (
  req: Request,
  res: Response,
  _next?: NextFunction
): Promise<void> => {
  try {
    const { events } = req.body as { events?: unknown };

    if (!Array.isArray(events)) {
      res.status(400).json({
        success: false,
        error: 'events must be an array',
      });
      return;
    }

    const validatedEvents = events.map((event: unknown) => revenueEventSchema.parse(event));

    const sdk = (req as any).sdk as { publisherId: string; appId: string } | undefined;
    if (sdk) {
      for (const ev of validatedEvents) {
        if (ev.publisher_id !== sdk.publisherId || ev.app_id !== sdk.appId) {
          res.status(403).json({
            success: false,
            error: 'publisher_id/app_id do not match SDK key scope',
          });
          return;
        }
      }
    }

    await analyticsService.recordRevenueEvents(validatedEvents);

    res.status(202).json({
      success: true,
      message: `Queued ${validatedEvents.length} revenue events`,
      count: validatedEvents.length,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      res.status(400).json({
        success: false,
        error: 'Invalid event data',
        details: error.errors,
      });
      return;
    }

    logger.error('Failed to record revenue', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to record revenue events',
    });
  }
};

/**
 * POST /api/v1/analytics/events/manifest-parse
 * Record SSAI manifest parse failures from SDKs/players.
 *
 * Best-effort only:
 * - Always returns quickly
 * - Does not block production paths
 */
export const recordManifestParseErrors = async (
  req: Request,
  res: Response,
  _next?: NextFunction
): Promise<void> => {
  try {
    const body = (req.body ?? {}) as any;
    const events = Array.isArray(body.events) ? body.events : [body];

    const parsedEvents = events.map((ev: unknown) => manifestParseErrorSchema.parse(ev));

    for (const ev of parsedEvents) {
      const observedAt = ev.timestamp ? new Date(ev.timestamp) : new Date();

      recordPerformanceMetric({
        observedAt,
        metricType: 'manifest_parse_error',
        metricValue: 1,
        publisherId: ev.publisher_id ?? null,
        requestId: ev.session_id ?? null,
        errorCode: ev.error_code ?? 'MANIFEST_PARSE_ERROR',
        errorMessage: ev.error_message ?? null,
        metadata: {
          source: 'sdk_parse_failure',
          protocol: ev.protocol,
          sdk_platform: ev.sdk_platform,
          sdk_version: ev.sdk_version,
          app_id: ev.app_id,
          ...(ev.metadata ?? {}),
        },
      });
    }

    res.status(202).json({
      success: true,
      message: `Accepted ${parsedEvents.length} manifest parse error event(s)`,
      count: parsedEvents.length,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      res.status(400).json({
        success: false,
        error: 'Invalid manifest parse event data',
        details: error.errors,
      });
      return;
    }

    logger.warn('[Analytics] Failed to accept manifest parse error event(s)', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to record manifest parse error events',
    });
  }
};

/**
 * GET /api/v1/analytics/buffer-stats
 * Get current buffer statistics (for monitoring)
 */
export const getBufferStats = (req: Request, res: Response): void => {
  const stats = analyticsService.getBufferStats();
  
  res.json({
    success: true,
    data: stats,
  });
};

// Viewability event schema for SDK_CHECKS 7.3
const viewabilityEventSchema = z.object({
  requestId: z.string(),
  impressionId: z.string(),
  placementId: z.string(),
  publisherId: z.string().optional(),
  platform: z.enum(['ios', 'android', 'android_tv', 'tvos', 'unity', 'web']),
  adFormat: z.enum(['banner', 'interstitial', 'rewarded', 'native', 'video']),
  omsdkAvailable: z.boolean(),
  omsdkSessionStarted: z.boolean().optional(),
  omsdkVersion: z.string().optional(),
  wasViewable: z.boolean(),
  measurable: z.boolean(),
  viewableTimeMs: z.number().optional(),
  totalDurationMs: z.number().optional(),
  viewablePercent: z.number().min(0).max(100).optional(),
  quartiles: z.object({
    start: z.boolean().optional(),
    firstQuartile: z.boolean().optional(),
    midpoint: z.boolean().optional(),
    thirdQuartile: z.boolean().optional(),
    complete: z.boolean().optional(),
  }).optional(),
  engagementEvents: z.array(z.string()).optional(),
  geometry: z.object({
    coveragePercent: z.number().optional(),
    overlappingCreatives: z.number().optional(),
    onScreenPercent: z.number().optional(),
  }).optional(),
  eventTimestamp: z.string().transform(s => new Date(s)),
});

/**
 * POST /api/v1/analytics/events/viewability
 * Record OMSDK/viewability events from SDK (SDK_CHECKS 7.3)
 */
export const recordViewability = async (
  req: Request,
  res: Response,
  next: NextFunction,
): Promise<void> => {
  try {
    const body = req.body;
    
    // Accept single event or array
    const events = Array.isArray(body.events) ? body.events : [body];
    
    const parsedEvents: viewabilityService.ViewabilityEvent[] = [];
    
    for (const event of events) {
      const parsed = viewabilityEventSchema.parse(event);
      parsedEvents.push(parsed as viewabilityService.ViewabilityEvent);
    }
    
    // Record events (buffered, non-blocking)
    viewabilityService.recordViewabilityEvents(parsedEvents);
    
    res.status(202).json({
      success: true,
      message: `Accepted ${parsedEvents.length} viewability event(s)`,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      res.status(400).json({
        success: false,
        error: 'Invalid viewability event data',
        details: error.errors,
      });
      return;
    }
    
    logger.error('Failed to record viewability events', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to record viewability events',
    });
  }
};

/**
 * GET /api/v1/analytics/viewability/:placementId
 * Get viewability summary for a placement
 */
export const getViewabilitySummary = async (
  req: Request,
  res: Response,
  next: NextFunction,
): Promise<void> => {
  try {
    const { placementId } = req.params;
    const startDate = req.query.startDate ? new Date(req.query.startDate as string) : undefined;
    const endDate = req.query.endDate ? new Date(req.query.endDate as string) : undefined;
    
    const summary = await viewabilityService.getViewabilitySummary(placementId, startDate, endDate);
    
    if (!summary) {
      res.status(404).json({
        success: false,
        error: 'No viewability data found for placement',
      });
      return;
    }
    
    res.json({
      success: true,
      data: summary,
    });
  } catch (error) {
    logger.error('Failed to get viewability summary', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to get viewability summary',
    });
  }
};

/**
 * GET /api/v1/analytics/omsdk-status
 * Get OMSDK status for publisher (shows in Console)
 */
export const getOmsdkStatus = async (
  req: Request,
  res: Response,
  next: NextFunction,
): Promise<void> => {
  try {
    const publisherId = (req as any).user?.id || req.query.publisherId as string;
    
    if (!publisherId) {
      res.status(400).json({
        success: false,
        error: 'Publisher ID required',
      });
      return;
    }
    
    const status = await viewabilityService.getOmsdkStatus(publisherId);
    
    res.json({
      success: true,
      data: status,
    });
  } catch (error) {
    logger.error('Failed to get OMSDK status', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to get OMSDK status',
    });
  }
};
