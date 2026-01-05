import { Request, Response, NextFunction } from 'express';
import type { ParsedQs } from 'qs';
import logger from '../utils/logger';
import reportingService from '../services/reportingService';
import { AppError } from '../middleware/errorHandler';

type QueryValue = string | string[] | ParsedQs | ParsedQs[] | undefined;

const getStringFromQueryValue = (value: QueryValue): string | undefined => {
  if (typeof value === 'string') return value;
  if (Array.isArray(value) && typeof value[0] === 'string') return value[0];
  return undefined;
};

const getStringFromObject = (value: unknown, key: string): string | undefined => {
  if (!value || typeof value !== 'object') return undefined;
  const entry = (value as Record<string, unknown>)[key];
  return typeof entry === 'string' ? entry : undefined;
};

function getIdempotencyKey(req: Request): { key: string | undefined; from: 'header' | 'body' | 'query' | 'none' } {
  const headerKey = req.header('Idempotency-Key');
  if (headerKey && typeof headerKey === 'string' && headerKey.trim().length > 0) {
    return { key: headerKey.trim(), from: 'header' };
  }
  // Deprecated fallbacks (body/query)
  const bodyKey = getStringFromObject(req.body, 'idempotencyKey');
  if (bodyKey && typeof bodyKey === 'string' && bodyKey.trim().length > 0) {
    return { key: bodyKey.trim(), from: 'body' };
  }
  const queryKey = getStringFromQueryValue((req.query as Record<string, QueryValue>).idempotencyKey);
  if (queryKey && typeof queryKey === 'string' && queryKey.trim().length > 0) {
    return { key: queryKey.trim(), from: 'query' };
  }
  return { key: undefined, from: 'none' };
}

function parseCurrencyAndLocale(req: Request): { currency: string; locale: string } {
  const currencyRaw = getStringFromQueryValue((req.query as Record<string, QueryValue>).currency);
  const currency = (currencyRaw ?? 'USD').toUpperCase().slice(0, 3);
  const acceptLang = req.headers['accept-language'];
  const locale = typeof acceptLang === 'string' && acceptLang.length > 0 ? acceptLang.split(',')[0] : 'en-US';
  return { currency, locale };
}

const DEFAULT_WINDOW_DAYS = 30;

const YMD_RE = /^\d{4}-\d{2}-\d{2}$/;

function parseDateParam(raw: string, kind: 'start' | 'end'): Date | undefined {
  const trimmed = raw.trim();
  if (YMD_RE.test(trimmed)) {
    const [y, m, d] = trimmed.split('-').map((s) => Number.parseInt(s, 10));
    if (!Number.isFinite(y) || !Number.isFinite(m) || !Number.isFinite(d)) return undefined;
    const utcStart = new Date(Date.UTC(y, m - 1, d));
    if (Number.isNaN(utcStart.getTime())) return undefined;
    // End dates supplied as YYYY-MM-DD are interpreted as inclusive days.
    // Our DB queries use an exclusive end boundary (observed_at < end), so shift to next-day midnight.
    return kind === 'end' ? new Date(utcStart.getTime() + 24 * 60 * 60 * 1000) : utcStart;
  }

  const dt = new Date(trimmed);
  if (Number.isNaN(dt.getTime())) return undefined;
  return dt;
}

function resolveDateRange(req: Request): { ok: true; start: Date; end: Date } | { ok: false; error: string } {
  const startDateRaw = getStringFromQueryValue((req.query as Record<string, QueryValue>).startDate);
  const endDateRaw = getStringFromQueryValue((req.query as Record<string, QueryValue>).endDate);
  const now = new Date();
  const defaultStart = new Date(now);
  defaultStart.setDate(now.getDate() - DEFAULT_WINDOW_DAYS);

  const start = startDateRaw ? parseDateParam(startDateRaw, 'start') : defaultStart;
  const end = endDateRaw ? parseDateParam(endDateRaw, 'end') : now;

  if (!start) {
    return { ok: false, error: 'Invalid startDate' };
  }
  if (!end) {
    return { ok: false, error: 'Invalid endDate' };
  }
  if (start > end) {
    return { ok: false, error: 'startDate must be <= endDate' };
  }

  return { ok: true, start, end };
}

/**
 * Get revenue summary
 */
export const getSummary = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      throw new AppError('Publisher context required', 400);
    }

    const { key, from } = getIdempotencyKey(req);
    if (from !== 'none' && from !== 'header') {
      logger.warn('[Revenue] Idempotency-Key fallback in use; prefer header', { from });
    }
    if (key) res.setHeader('Idempotency-Key', key);

    const range = resolveDateRange(req);
    if (!range.ok) {
      res.status(400).json({ success: false, error: range.error });
      return;
    }

    const { start, end } = range;
    const { currency, locale } = parseCurrencyAndLocale(req);
    const stats = await reportingService.getRevenueStats(publisherId, start, end);

    const summary = {
      totalRevenue: stats.totalRevenue,
      totalImpressions: stats.totalImpressions,
      totalClicks: stats.totalClicks,
      averageEcpm: stats.ecpm,
      averageFillRate: stats.fillRate,
      periodStart: start.toISOString(),
      periodEnd: end.toISOString(),
      currency,
    };

    res.json({
      success: true,
      data: summary,
      meta: { currency, locale, idempotencyKey: key },
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get revenue time series data
 */
export const getTimeSeries = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      throw new AppError('Publisher context required', 400);
    }

    const { startDate, endDate, granularity = 'day' } = req.query as Record<string, QueryValue> & { granularity?: QueryValue };
    const { key, from } = getIdempotencyKey(req);
    if (from !== 'none' && from !== 'header') {
      logger.warn('[Revenue] Idempotency-Key fallback in use; prefer header', { from });
    }
    if (key) res.setHeader('Idempotency-Key', key);

    const validGranularities = new Set(['hour', 'day']);
    const gran = String(granularity);
    if (!validGranularities.has(gran)) {
      res.status(400).json({ success: false, error: 'Invalid granularity. Use hour|day.' });
      return;
    }

    const range = resolveDateRange(req);
    if (!range.ok) {
      res.status(400).json({ success: false, error: range.error });
      return;
    }

    const { start, end } = range;
    const { currency, locale } = parseCurrencyAndLocale(req);
    const data = await reportingService.getTimeSeriesData(publisherId, start, end, gran as 'hour' | 'day');

    const timeSeries = data.map((point) => ({
      date: point.timestamp,
      revenue: point.revenue,
      impressions: point.impressions,
      clicks: point.clicks,
      ecpm: point.ecpm,
      fillRate: 0, // placeholder until request volume metric available
    }));

    res.json({
      success: true,
      data: timeSeries,
      meta: {
        startDate: startDate ? String(startDate) : start.toISOString(),
        endDate: endDate ? String(endDate) : end.toISOString(),
        granularity: gran,
        currency,
        locale,
        idempotencyKey: key,
      },
    });
  } catch (error) {
    next(error);
  }
};
