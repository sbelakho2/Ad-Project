import {
  fetchFraudStats,
  fetchRecentFraudAlerts,
  fetchFraudTypeBreakdown,
  fetchFraudStatsV2,
  fetchRecentFraudAlertsV2,
} from '../repositories/fraudRepository';
import logger from '../utils/logger';

type CacheEntry<T> = { value: T; expiresAt: number };
const ttlMs = Math.max(1000, parseInt(process.env.FRAUD_CACHE_TTL_MS || '15000', 10));
const cache = new Map<string, CacheEntry<unknown>>();
let hits = 0;
let misses = 0;

const cacheKey = (scope: string, args: Record<string, unknown>) =>
  `${scope}:${Object.entries(args)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([k, v]) => `${k}=${String(v)}`)
    .join('&')}`;

const fromCache = <T>(key: string): T | null => {
  const now = Date.now();
  const e = cache.get(key);
  if (e && e.expiresAt > now) {
    hits++;
    return e.value as T;
  }
  if (e) cache.delete(key);
  misses++;
  return null;
};

const putCache = <T>(key: string, value: T): T => {
  cache.set(key, { value, expiresAt: Date.now() + ttlMs });
  return value;
};

export const fraudCacheStats = () => ({ size: cache.size, ttlMs, hits, misses });

export const resetFraudCache = (): void => {
  cache.clear();
  hits = 0;
  misses = 0;
};

export const getFraudStatistics = async (publisherId: string): Promise<ReturnType<typeof shapeStats>> => {
  const key = cacheKey('stats', { publisherId });
  const cached = fromCache<ReturnType<typeof shapeStats>>(key);
  if (cached) return cached;

  const stats = await fetchFraudStats(publisherId);
  const typeBreakdown = await fetchFraudTypeBreakdown(publisherId);
  const shaped = shapeStats(stats, typeBreakdown);
  return putCache(key, shaped);
};

export const getFraudStatisticsV2 = async (
  publisherId: string,
  opts: { window?: string } = {}
): Promise<Awaited<ReturnType<typeof fetchFraudStatsV2>>> => {
  const key = cacheKey('statsV2', { publisherId, window: opts.window ?? '24h' });
  const cached = fromCache<Awaited<ReturnType<typeof fetchFraudStatsV2>>>(key);
  if (cached) return cached;
  const stats = await fetchFraudStatsV2(publisherId, { window: opts.window });
  return putCache(key, stats);
};

export const listFraudAlerts = async (publisherId: string, limit: number) => {
  const safeLimit = Number.isFinite(limit) ? Math.min(Math.max(1, Math.floor(limit)), 200) : 50;
  const key = cacheKey('alerts', { publisherId, limit: safeLimit });
  const cached = fromCache<Awaited<ReturnType<typeof fetchRecentFraudAlerts>>>(key);
  if (cached) return cached;
  const rows = await fetchRecentFraudAlerts(publisherId, safeLimit).catch((e) => {
    logger.warn('fraudDetection.listFraudAlerts: repository error', { error: e });
    return [] as Awaited<ReturnType<typeof fetchRecentFraudAlerts>>;
  });
  return putCache(key, rows);
};

export const listFraudAlertsV2 = async (
  publisherId: string,
  limit: number,
  opts: { window?: string } = {}
) => {
  const safeLimit = Number.isFinite(limit) ? Math.min(Math.max(1, Math.floor(limit)), 200) : 50;
  const key = cacheKey('alertsV2', { publisherId, limit: safeLimit, window: opts.window ?? '24h' });
  const cached = fromCache<Awaited<ReturnType<typeof fetchRecentFraudAlertsV2>>>(key);
  if (cached) return cached;
  const rows = await fetchRecentFraudAlertsV2(publisherId, safeLimit, { window: opts.window }).catch((e) => {
    logger.warn('fraudDetection.listFraudAlertsV2: repository error', { error: e });
    return [] as Awaited<ReturnType<typeof fetchRecentFraudAlertsV2>>;
  });
  return putCache(key, rows);
};

export const getFraudByType = async (publisherId: string): Promise<Array<{ type: string; count: number; blockedRevenue: number }>> => {
  const key = cacheKey('byType', { publisherId });
  const cached = fromCache<Array<{ type: string; count: number; blockedRevenue: number }>>(key);
  if (cached) return cached;
  const breakdown = await fetchFraudTypeBreakdown(publisherId);
  const rows = breakdown.map((row) => ({
    type: row.type,
    count: safeInt(row.alerts),
    blockedRevenue: safeMoney(row.blockedRevenue),
  }));
  return putCache(key, rows);
};

// ----- helpers -----
const safeMoney = (n: any): number => {
  const v = typeof n === 'number' ? n : Number(n);
  return Number.isFinite(v) ? Number(v.toFixed(2)) : 0;
};

const safeInt = (n: any): number => {
  const v = typeof n === 'number' ? n : Number(n);
  return Number.isFinite(v) ? Math.max(0, Math.floor(v)) : 0;
};

const shapeStats = (
  stats: {
    totalAlerts: number;
    blockedRevenue: number;
    highSeverity: number;
    mediumSeverity: number;
    lastDetectionAt?: string;
  },
  typeBreakdown: Array<{ type: string; alerts: number; blockedRevenue: number }>
) => {
  const total = safeInt(stats.totalAlerts);
  const high = safeInt(stats.highSeverity);
  const med = safeInt(stats.mediumSeverity);
  return {
    totalDetected: total,
    blockedRevenue: safeMoney(stats.blockedRevenue),
    detectionRate: total > 0 ? Number(((high / total) * 100).toFixed(2)) : 0,
    avgFraudScore: total > 0 ? Number(((high * 1 + med * 0.5) / total).toFixed(2)) : 0,
    topTypes: typeBreakdown.map((row) => ({
      type: row.type,
      count: safeInt(row.alerts),
      percentage: total > 0 ? Number(((safeInt(row.alerts) / total) * 100).toFixed(1)) : 0,
    })),
    lastDetectionAt: stats.lastDetectionAt,
  };
};
