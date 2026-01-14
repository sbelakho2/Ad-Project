import { Request, Response, NextFunction } from 'express';
import crypto from 'crypto';
import logger from '../utils/logger';
import { AuctionRequestBody } from '../schemas/rtb';
import { runAuction } from '../services/rtb/orchestrator';
import { getAllAdapters } from '../services/rtb/adapterRegistry';
import config from '../config/index';
import * as breaker from '../utils/redisCircuitBreaker';
import * as idempotency from '../services/rtb/auctionIdempotency';

/**
 * POST /api/v1/rtb/bid
 * OpenRTB 2.6 compliant bid request endpoint
 * Supports waterfall fallback via query parameter: ?waterfall=true
 */
export const requestBid = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const body = req.body as AuctionRequestBody;
    const enabled = process.env.ENABLE_PRODUCTION_RTB === '1';

    // Ensure requestId is present (generate if missing)
    const requestId = body.requestId || crypto.randomUUID?.() || `req-${Date.now()}`;
    const publisherId = (req as any).sdk?.publisherId;

    if (!publisherId) {
      res.status(401).json({ success: false, error: 'Unauthorized: Publisher ID missing' });
      return;
    }

    // Check idempotency cache for duplicate request
    if (idempotency.isIdempotencyEnabled()) {
      const cached = await idempotency.getCachedAuctionResult(requestId);
      if (cached) {
        logger.debug('Returning cached auction result', { requestId, landscapeId: cached.landscapeId });
        if (!cached.success || !cached.response) {
          if (cached.reason === 'NO_BID') {
            res.status(204).send();
            return;
          }
          res.status(400).json({ success: false, error: 'Auction failed', reason: cached.reason, landscapeId: cached.landscapeId });
          return;
        }
        res.json(cached.response);
        return;
      }
    }

    const proto = (req.headers['x-forwarded-proto'] as string) || (req.protocol || 'http');
    const host = (req.headers['x-forwarded-host'] as string) || req.get('host');
    const baseUrl = `${proto}://${host}`;

    const migration = body.signal?.migration
      ? {
          experimentId: body.signal.migration.experiment_id,
          arm: body.signal.migration.arm,
          assignmentTs: body.signal.migration.assignment_ts,
          mirrorPercent: body.signal.migration.mirror_percent,
          mode: body.signal.migration.mode,
        }
      : undefined;

    const result = await runAuction({
      requestId,
      publisherId,
      placementId: body.placementId,
      adFormat: body.adFormat,
      floorCpm: body.floorCpm ?? 0,
      device: body.device,
      user: body.user,
      app: body.app,
      consent: body.consent && {
        gdpr: body.consent.gdpr,
        gdprConsent: body.consent.gdpr_consent,
        usPrivacy: body.consent.us_privacy,
        coppa: body.consent.coppa,
      },
      signal: body.signal,
      migration,
    }, baseUrl);

    // Cache result for idempotency
    if (idempotency.isIdempotencyEnabled()) {
      await idempotency.cacheAuctionResult(requestId, {
        landscapeId: result.landscapeId,
        success: result.success,
        response: result.response,
        reason: result.reason,
      });
    }

    if (!result.success || !result.response) {
      if (result.reason === 'CONTROL_ARM' || result.reason === 'SHADOW_MODE') {
        res.status(204).send();
        return;
      }
      // Provide structured error response to aid debugging and instrumentation
      const reason = result.reason || 'UNKNOWN';
      res.status(400).json({ success: false, error: 'Auction failed', reason, landscapeId: result.landscapeId });
      return;
    }

    res.json(result.response);
  } catch (error) {
    logger.error('RTB bid request failed', { error });
    next(error);
  }
};

/**
 * GET /api/v1/rtb/status
 * Get RTB engine status and circuit breaker stats
 */
export const getStatus = async (
  _req: Request,
  res: Response
): Promise<void> => {
  try {
    const adapters = getAllAdapters();
    const adapterInfos = adapters.map(a => ({ name: a.name, supports: a.supports, timeoutMs: a.timeoutMs }));
    let circuitSummary: Record<string, { open: boolean; failuresWindow: number }> = {};
    if (config.redisBreakersEnabled) {
      try {
        circuitSummary = await breaker.getSummary(adapters.map(a => a.name));
      } catch (e) {
        // If Redis not available or summary fails, leave empty
        circuitSummary = {};
      }
    }

    res.json({
      success: true,
      data: {
        adapters: adapterInfos,
        circuitBreakers: circuitSummary,
        waterfall: {},
        breakersEnabled: config.redisBreakersEnabled,
        status: 'operational',
      },
    });
  } catch (error) {
    logger.error('Failed to get RTB status', { error });
    res.status(500).json({ success: false, error: 'Failed to get status' });
  }
};
