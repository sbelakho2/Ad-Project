import { Request, Response, NextFunction } from 'express';
import { AppError } from '../middleware/errorHandler';
import {
  getFraudStatistics,
  getFraudStatisticsV2,
  listFraudAlerts,
  listFraudAlertsV2,
  getFraudByType,
} from '../services/fraudDetection';

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
 * Get fraud detection statistics
 */
export const getStats = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const windowParam = parseQueryParam(req.query.window);
    const stats = await getFraudStatisticsV2(publisherId, { window: windowParam });

    res.json({
      success: true,
      data: stats,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get fraud alerts
 */
export const getAlerts = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const limitParam = parseQueryParam(req.query.limit);
    const limit = limitParam ? Number(limitParam) : 10;

    const windowParam = parseQueryParam(req.query.window);

    if (!Number.isFinite(limit) || limit <= 0) {
      throw new AppError('Invalid limit parameter', 400);
    }

    const alerts = await listFraudAlertsV2(publisherId, limit, { window: windowParam });

    res.json({
      success: true,
      data: { alerts },
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get fraud breakdown by type
 */
export const getByType = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Missing publisher context', 401);
    }

    const types = await getFraudByType(publisherId);

    res.json({
      success: true,
      data: types,
    });
  } catch (error) {
    next(error);
  }
};
