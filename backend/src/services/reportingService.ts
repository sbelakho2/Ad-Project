/**
 * Reporting Service
 *
 * Queries Postgres analytics fact tables for reporting data
 */

import type { QueryResultRow } from 'pg';
import { query as pgQuery } from '../utils/postgres';
import logger from '../utils/logger';

const REPORTING_SLOW_QUERY_MS = parseInt(process.env.REPORTING_SLOW_QUERY_MS || '200', 10);
const REPORTING_CAPTURE_EXPLAIN = process.env.REPORTING_CAPTURE_EXPLAIN !== '0';

// Prefer rollups for history, but compute the most recent window from raw fact tables
// to avoid dashboards appearing stale when rollups run infrequently.
const REPORTING_RECENT_RAW_DAYS = Math.max(0, parseInt(process.env.REPORTING_RECENT_RAW_DAYS || '2', 10));

const formatUtcDate = (d: Date): string => d.toISOString().slice(0, 10);

const startOfUtcDayFromYmd = (ymd: string): Date => new Date(`${ymd}T00:00:00.000Z`);

const maxDate = (a: Date, b: Date): Date => (a.getTime() >= b.getTime() ? a : b);
const minDate = (a: Date, b: Date): Date => (a.getTime() <= b.getTime() ? a : b);

const rollupTableExists = async (tableName: string): Promise<boolean> => {
  try {
    const res = await pgQuery<{ exists: boolean }>(
      `
        SELECT EXISTS(
          SELECT 1
          FROM information_schema.tables
          WHERE table_schema = 'public'
            AND table_name = $1
        ) AS exists
      `,
      [tableName],
      { replica: true, label: 'REPORTING_SCHEMA_CHECK' }
    );
    return Boolean(res.rows?.[0]?.exists);
  } catch {
    return false;
  }
};

const runQuery = async <T extends QueryResultRow = QueryResultRow>(
  sql: string,
  params: ReadonlyArray<unknown>,
  label?: string
): Promise<T[]> => {
  const queryLabel = label ?? 'REPORTING';
  const start = Date.now();
  const { rows } = await pgQuery<T>(sql, params, { replica: true, label: queryLabel });
  const durationMs = Date.now() - start;

  if (durationMs > REPORTING_SLOW_QUERY_MS) {
    logger.warn('Slow reporting query detected', { label: queryLabel, durationMs });
    if (REPORTING_CAPTURE_EXPLAIN) {
      try {
        const explainSql = `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)\n${sql}`;
        const plan = await pgQuery<{ 'QUERY PLAN': unknown }>(explainSql, params, {
          replica: true,
          label: `${queryLabel}_EXPLAIN`,
        });
        logger.warn('Reporting query plan', {
          label: queryLabel,
          durationMs,
          plan: plan.rows?.[0]?.['QUERY PLAN'] ?? plan.rows,
        });
      } catch (planError) {
        logger.error('Failed to capture reporting query plan', {
          label: queryLabel,
          durationMs,
          error: (planError as Error).message,
        });
      }
    }
  }

  return rows;
};

export interface RevenueStats {
  totalRevenue: number;
  totalImpressions: number;
  totalClicks: number;
  ecpm: number;
  ctr: number;
  fillRate: number;
}

export interface TimeSeriesDataPoint {
  timestamp: string;
  revenue: number;
  impressions: number;
  clicks: number;
  ecpm: number;
}

export interface AdapterPerformance {
  adapterId: string;
  adapterName: string;
  revenue: number;
  impressions: number;
  clicks: number;
  ecpm: number;
  ctr: number;
  avgLatency: number;
}

export interface CountryBreakdown {
  countryCode: string;
  revenue: number;
  impressions: number;
  ecpm: number;
}

export interface AdapterHealthScore {
  adapterId: string;
  adapterName: string;
  healthScore: number; // 0-100
  uptime: number; // percentage
  errorRate: number;
  avgResponseTime: number;
  fillRate: number;
  revenueShare: number;
  status: 'healthy' | 'degraded' | 'critical' | 'offline';
  lastIssue?: string;
  issuesLast24h: number;
}

export interface FraudMetrics {
  totalRequests: number;
  fraudRequests: number;
  fraudRate: number;
  givtDetections: number;
  sivtDetections: number;
  mlDetections: number;
  anomalyDetections: number;
  blockedRevenue: number;
  topFraudTypes: Array<{ type: string; count: number }>;
}

export interface QualityMetrics {
  viewabilityRate: number;
  completionRate: number;
  clickThroughRate: number;
  invalidTrafficRate: number;
  brandSafetyScore: number;
  userExperienceScore: number;
  anrRate: number;
  crashRate: number;
}

export interface RevenueProjection {
  date: string;
  projectedRevenue: number;
  lowerBound: number;
  upperBound: number;
  confidence: number;
}

export interface CohortMetrics {
  cohortDate: string;
  userCount: number;
  day0Revenue: number;
  day1Revenue: number;
  day7Revenue: number;
  day30Revenue: number;
  ltv: number;
  retentionDay1: number;
  retentionDay7: number;
  retentionDay30: number;
  arpu: number;
}

export interface AnomalyAlert {
  id: string;
  timestamp: string;
  type: 'revenue_drop' | 'fraud_spike' | 'performance_degradation' | 'traffic_anomaly';
  severity: 'low' | 'medium' | 'high' | 'critical';
  metric: string;
  currentValue: number;
  expectedValue: number;
  deviation: number;
  description: string;
}

export interface ReportingService {
  getRevenueStats(publisherId: string, startDate: Date, endDate: Date): Promise<RevenueStats>;
  getTimeSeriesData(publisherId: string, startDate: Date, endDate: Date, granularity: 'hour' | 'day'): Promise<TimeSeriesDataPoint[]>;
  getAdapterPerformance(publisherId: string, startDate: Date, endDate: Date): Promise<AdapterPerformance[]>;
  getCountryBreakdown(publisherId: string, startDate: Date, endDate: Date, limit: number): Promise<CountryBreakdown[]>;
  
  // Advanced analytics methods
  getAdapterHealthScores(publisherId: string): Promise<AdapterHealthScore[]>;
  getFraudMetrics(publisherId: string, startDate: Date, endDate: Date): Promise<FraudMetrics>;
  getQualityMetrics(publisherId: string, startDate: Date, endDate: Date): Promise<QualityMetrics>;
  getRevenueProjections(publisherId: string, days: number): Promise<RevenueProjection[]>;
  getCohortAnalysis(publisherId: string, startDate: Date, endDate: Date): Promise<CohortMetrics[]>;
  getAnomalies(publisherId: string, hours: number): Promise<AnomalyAlert[]>;
}

class ReportingServiceImpl implements ReportingService {
  /**
   * Get overall revenue statistics for a publisher
   */
  async getRevenueStats(
    publisherId: string,
    startDate: Date,
    endDate: Date
  ): Promise<RevenueStats> {
    try {
      // Prefer rollups when present (faster + allows raw fact retention to be reduced safely).
      if (await rollupTableExists('analytics_metrics_rollups')) {
        const splitDay = formatUtcDate(
          new Date(endDate.getTime() - REPORTING_RECENT_RAW_DAYS * 24 * 60 * 60 * 1000)
        );
        const splitTs = startOfUtcDayFromYmd(splitDay);

        const rollupEndExclusive = minDate(endDate, splitTs);
        const rollupStartDay = formatUtcDate(startDate);
        const rollupEndDay = formatUtcDate(rollupEndExclusive);

        let rollupRevenue = 0;
        let rollupImpressions = 0;
        let rollupClicks = 0;
        let rollupFilled = 0;

        if (rollupStartDay < rollupEndDay) {
          // Use pre-aggregated publisher-level summary for fast dashboard queries
          const rollupSql = `
            SELECT
              COALESCE(SUM(total_revenue), 0) AS total_revenue,
              COALESCE(SUM(impression_count), 0) AS total_impressions,
              COALESCE(SUM(click_count), 0) AS total_clicks,
              COALESCE(SUM(filled_count), 0) AS total_filled
            FROM analytics_publisher_daily_summary
            WHERE publisher_id = $1::text
              AND bucket_date >= $2::date
              AND bucket_date < $3::date
          `;

          const rollupResult = await runQuery<{
            total_revenue: string;
            total_impressions: string;
            total_clicks: string;
            total_filled: string;
          }>(rollupSql, [publisherId, rollupStartDay, rollupEndDay], 'REPORTING_OVERVIEW_ROLLUPS');

          const rollupRow = rollupResult[0] ?? {
            total_revenue: '0',
            total_impressions: '0',
            total_clicks: '0',
            total_filled: '0',
          };

          rollupRevenue = Number.parseFloat(rollupRow.total_revenue);
          rollupImpressions = Number.parseInt(rollupRow.total_impressions);
          rollupClicks = Number.parseInt(rollupRow.total_clicks);
          rollupFilled = Number.parseInt(rollupRow.total_filled);
        }

        const rawStart = maxDate(startDate, splitTs);
        let rawRevenue = 0;
        let rawImpressions = 0;
        let rawClicks = 0;
        let rawFilled = 0;

        if (rawStart.getTime() < endDate.getTime()) {
          const rawSql = `
            WITH
              imp AS (
                SELECT
                  COUNT(*)::bigint AS impressions,
                  COUNT(*) FILTER (WHERE filled IS TRUE)::bigint AS filled
                FROM analytics_impressions
                WHERE publisher_id = $1::uuid
                  AND observed_at >= $2
                  AND observed_at < $3
                  AND is_test_mode = false
              ),
              clk AS (
                SELECT COUNT(*)::bigint AS clicks
                FROM analytics_clicks
                WHERE publisher_id = $1::text
                  AND observed_at >= $2
                  AND observed_at < $3
                  AND is_test_mode = false
              ),
              rev AS (
                SELECT COALESCE(SUM(revenue_usd), 0) AS revenue
                FROM analytics_revenue_events
                WHERE publisher_id = $1::uuid
                  AND observed_at >= $2
                  AND observed_at < $3
                  AND is_test_mode = false
              )
            SELECT
              rev.revenue::text AS revenue,
              imp.impressions::text AS impressions,
              clk.clicks::text AS clicks,
              imp.filled::text AS filled
            FROM imp
            CROSS JOIN clk
            CROSS JOIN rev
          `;

          const rawResult = await runQuery<{
            revenue: string;
            impressions: string;
            clicks: string;
            filled: string;
          }>(rawSql, [publisherId, rawStart, endDate], 'REPORTING_OVERVIEW_RAW_RECENT');

          const rawRow = rawResult[0] ?? { revenue: '0', impressions: '0', clicks: '0', filled: '0' };
          rawRevenue = Number.parseFloat(rawRow.revenue);
          rawImpressions = Number.parseInt(rawRow.impressions);
          rawClicks = Number.parseInt(rawRow.clicks);
          rawFilled = Number.parseInt(rawRow.filled);
        }

        const totalRevenue = (Number.isFinite(rollupRevenue) ? rollupRevenue : 0) + (Number.isFinite(rawRevenue) ? rawRevenue : 0);
        const totalImpressions = (Number.isFinite(rollupImpressions) ? rollupImpressions : 0) + (Number.isFinite(rawImpressions) ? rawImpressions : 0);
        const totalClicks = (Number.isFinite(rollupClicks) ? rollupClicks : 0) + (Number.isFinite(rawClicks) ? rawClicks : 0);
        const totalFilled = (Number.isFinite(rollupFilled) ? rollupFilled : 0) + (Number.isFinite(rawFilled) ? rawFilled : 0);
        const ecpm = totalImpressions > 0 ? (totalRevenue / totalImpressions) * 1000 : 0;
        const ctr = totalImpressions > 0 ? (totalClicks / totalImpressions) * 100 : 0;
        const fillRate = totalImpressions > 0 ? (totalFilled / totalImpressions) * 100 : 0;

        return {
          totalRevenue: Number.isFinite(totalRevenue) ? totalRevenue : 0,
          totalImpressions: Number.isFinite(totalImpressions) ? totalImpressions : 0,
          totalClicks: Number.isFinite(totalClicks) ? totalClicks : 0,
          ecpm,
          ctr,
          fillRate,
        };
      }

      const sql = `
        SELECT
          COALESCE(SUM(r.revenue_usd), 0) AS total_revenue,
          COALESCE(COUNT(DISTINCT r.impression_id), 0) AS total_impressions,
          COALESCE(COUNT(*) FILTER (WHERE r.revenue_type = 'click'), 0) AS total_clicks
        FROM analytics_revenue_events r
        WHERE r.publisher_id = $1::uuid
          AND r.observed_at >= $2
          AND r.observed_at < $3
          AND r.is_test_mode = false
      `;

      const result = await runQuery<{
        total_revenue: string;
        total_impressions: string;
        total_clicks: string;
      }>(sql, [publisherId, startDate, endDate]);

      if (result.length === 0) {
        return {
          totalRevenue: 0,
          totalImpressions: 0,
          totalClicks: 0,
          ecpm: 0,
          ctr: 0,
          fillRate: 0,
        };
      }

      const row = result[0];
      const totalImpressions = Number.parseInt(row.total_impressions);
      const totalClicks = Number.parseInt(row.total_clicks);
      const totalRevenue = Number.parseFloat(row.total_revenue);
      const ecpm = totalImpressions > 0 ? (totalRevenue / totalImpressions) * 1000 : 0;
      const ctr = totalImpressions > 0 ? (totalClicks / totalImpressions) * 100 : 0;

      return {
        totalRevenue: Number.isFinite(totalRevenue) ? totalRevenue : 0,
        totalImpressions: Number.isFinite(totalImpressions) ? totalImpressions : 0,
        totalClicks: Number.isFinite(totalClicks) ? totalClicks : 0,
        ecpm,
        ctr,
        // Leave fillRate as 0 until request-volume metric is integrated
        fillRate: 0,
      };
    } catch (error) {
      logger.error('Failed to get revenue stats', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get time series data for charting
   */
  async getTimeSeriesData(
    publisherId: string,
    startDate: Date,
    endDate: Date,
    granularity: 'hour' | 'day' = 'day'
  ): Promise<TimeSeriesDataPoint[]> {
    try {
      if (granularity === 'day' && (await rollupTableExists('analytics_metrics_rollups'))) {
        const splitDay = formatUtcDate(
          new Date(endDate.getTime() - REPORTING_RECENT_RAW_DAYS * 24 * 60 * 60 * 1000)
        );
        const splitTs = startOfUtcDayFromYmd(splitDay);

        const rollupEndExclusive = minDate(endDate, splitTs);
        const rollupStartDay = formatUtcDate(startDate);
        const rollupEndDay = formatUtcDate(rollupEndExclusive);

        const points: TimeSeriesDataPoint[] = [];

        if (rollupStartDay < rollupEndDay) {
          // Use pre-aggregated publisher-level summary for fast dashboard queries
          const rollupSql = `
            SELECT
              bucket_date AS bucket,
              COALESCE(SUM(total_revenue), 0) AS revenue,
              COALESCE(SUM(impression_count), 0) AS impressions,
              COALESCE(SUM(click_count), 0) AS clicks
            FROM analytics_publisher_daily_summary
            WHERE publisher_id = $1::text
              AND bucket_date >= $2::date
              AND bucket_date < $3::date
            GROUP BY bucket
            ORDER BY bucket ASC
          `;

          const rollupResult = await runQuery<{
            bucket: Date | string;
            revenue: string;
            impressions: string;
            clicks: string;
          }>(rollupSql, [publisherId, rollupStartDay, rollupEndDay], 'REPORTING_TIMESERIES_ROLLUPS');

          points.push(
            ...rollupResult.map(row => {
              const impressions = Number.parseInt(row.impressions);
              const revenue = Number.parseFloat(row.revenue);
              const clicks = Number.parseInt(row.clicks);
              // Handle both Date objects (from node-postgres) and strings
              const bucketDate = row.bucket instanceof Date
                ? row.bucket
                : new Date(`${row.bucket}T00:00:00.000Z`);
              return {
                timestamp: bucketDate.toISOString(),
                revenue,
                impressions,
                clicks,
                ecpm: impressions > 0 ? (revenue / impressions) * 1000 : 0,
              };
            })
          );
        }

        const rawStart = maxDate(startDate, splitTs);
        if (rawStart.getTime() < endDate.getTime()) {
          const rawSql = `
            WITH
              imp AS (
                SELECT
                  date_trunc('day', observed_at) AS bucket,
                  COUNT(*)::bigint AS impressions
                FROM analytics_impressions
                WHERE publisher_id = $1::uuid
                  AND observed_at >= $2
                  AND observed_at < $3
                  AND is_test_mode = false
                GROUP BY 1
              ),
              clk AS (
                SELECT
                  date_trunc('day', observed_at) AS bucket,
                  COUNT(*)::bigint AS clicks
                FROM analytics_clicks
                WHERE publisher_id = $1::text
                  AND observed_at >= $2
                  AND observed_at < $3
                  AND is_test_mode = false
                GROUP BY 1
              ),
              rev AS (
                SELECT
                  date_trunc('day', observed_at) AS bucket,
                  COALESCE(SUM(revenue_usd), 0) AS revenue
                FROM analytics_revenue_events
                WHERE publisher_id = $1::uuid
                  AND observed_at >= $2
                  AND observed_at < $3
                  AND is_test_mode = false
                GROUP BY 1
              ),
              ic AS (
                SELECT
                  COALESCE(imp.bucket, clk.bucket) AS bucket,
                  COALESCE(imp.impressions, 0)::bigint AS impressions,
                  COALESCE(clk.clicks, 0)::bigint AS clicks
                FROM imp
                FULL OUTER JOIN clk
                  ON clk.bucket = imp.bucket
              )
            SELECT
              COALESCE(ic.bucket, rev.bucket) AS bucket,
              COALESCE(rev.revenue, 0) AS revenue,
              COALESCE(ic.impressions, 0) AS impressions,
              COALESCE(ic.clicks, 0) AS clicks
            FROM ic
            FULL OUTER JOIN rev
              ON rev.bucket = ic.bucket
            ORDER BY bucket ASC
          `;

          const rawResult = await runQuery<{
            bucket: Date | string;
            revenue: string;
            impressions: string;
            clicks: string;
          }>(rawSql, [publisherId, rawStart, endDate], 'REPORTING_TIMESERIES_RAW_RECENT');

          points.push(
            ...rawResult.map(row => {
              const impressions = Number.parseInt(row.impressions);
              const revenue = Number.parseFloat(row.revenue);
              const clicks = Number.parseInt(row.clicks);
              // Handle both Date objects and strings from PostgreSQL
              const bucketDate = row.bucket instanceof Date ? row.bucket : new Date(row.bucket);
              return {
                timestamp: bucketDate.toISOString(),
                revenue,
                impressions,
                clicks,
                ecpm: impressions > 0 ? (revenue / impressions) * 1000 : 0,
              };
            })
          );
        }

        return points.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
      }

      const bucket = granularity === 'hour' ? 'hour' : 'day';
      const sql = `
        SELECT
          date_trunc('${bucket}', r.observed_at) AS bucket,
          SUM(r.revenue_usd) AS revenue,
          COUNT(DISTINCT r.impression_id) AS impressions,
          COUNT(*) FILTER (WHERE r.revenue_type = 'click') AS clicks
        FROM analytics_revenue_events r
        WHERE r.publisher_id = $1::uuid
          AND r.observed_at >= $2
          AND r.observed_at < $3
          AND r.is_test_mode = false
        GROUP BY bucket
        ORDER BY bucket ASC
      `;

      const result = await runQuery<{
        bucket: Date | string;
        revenue: string;
        impressions: string;
        clicks: string;
      }>(sql, [publisherId, startDate, endDate]);

      return result.map(row => {
        // Handle both Date objects and strings from PostgreSQL
        const bucketDate = row.bucket instanceof Date ? row.bucket : new Date(row.bucket);
        return {
          timestamp: bucketDate.toISOString(),
          revenue: parseFloat(row.revenue),
          impressions: parseInt(row.impressions),
          clicks: parseInt(row.clicks),
          ecpm: parseInt(row.impressions) > 0
            ? (parseFloat(row.revenue) / parseInt(row.impressions)) * 1000
            : 0,
        };
      });
    } catch (error) {
      logger.error('Failed to get time series data', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get performance breakdown by adapter
   */
  async getAdapterPerformance(
    publisherId: string,
    startDate: Date,
    endDate: Date
  ): Promise<AdapterPerformance[]> {
    try {
      const sql = `
        SELECT
          r.adapter_id,
          COALESCE(r.adapter_name, 'unknown') AS adapter_name,
          SUM(r.revenue_usd) AS revenue,
          COUNT(DISTINCT r.impression_id) AS impressions,
          COUNT(*) FILTER (WHERE r.revenue_type = 'click') AS clicks,
          AVG(i.latency_ms) AS avg_latency
        FROM analytics_revenue_events r
        LEFT JOIN analytics_impressions i ON r.impression_id = i.event_id
        WHERE r.publisher_id = $1::uuid
          AND r.observed_at >= $2
          AND r.observed_at < $3
          AND r.is_test_mode = false
        GROUP BY r.adapter_id, r.adapter_name
        ORDER BY revenue DESC
      `;

      const result = await runQuery<{
        adapter_id: string;
        adapter_name: string;
        revenue: string;
        impressions: string;
        clicks: string;
        avg_latency: string | null;
      }>(sql, [publisherId, startDate, endDate]);

      return result.map(row => ({
        adapterId: row.adapter_id,
        adapterName: row.adapter_name,
        revenue: parseFloat(row.revenue),
        impressions: parseInt(row.impressions),
        clicks: parseInt(row.clicks),
        ecpm: parseInt(row.impressions) > 0
          ? (parseFloat(row.revenue) / parseInt(row.impressions)) * 1000
          : 0,
        ctr: parseInt(row.impressions) > 0
          ? (parseInt(row.clicks) / parseInt(row.impressions)) * 100
          : 0,
        avgLatency: row.avg_latency ? parseFloat(row.avg_latency) : 0,
      }));
    } catch (error) {
      logger.error('Failed to get adapter performance', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get revenue breakdown by country
   */
  async getCountryBreakdown(
    publisherId: string,
    startDate: Date,
    endDate: Date,
    limit: number = 10
  ): Promise<CountryBreakdown[]> {
    try {
      if (await rollupTableExists('analytics_metrics_rollups')) {
        const sql = `
          SELECT
            country_code,
            COALESCE(SUM(total_revenue), 0) AS revenue,
            COALESCE(SUM(impression_count), 0) AS impressions
          FROM analytics_metrics_rollups
          WHERE publisher_id = $1::text
            AND bucket_date >= $2::date
            AND bucket_date < $3::date
          GROUP BY country_code
          ORDER BY revenue DESC
          LIMIT $4
        `;

        const result = await runQuery<{
          country_code: string;
          revenue: string;
          impressions: string;
        }>(sql, [publisherId, formatUtcDate(startDate), formatUtcDate(endDate), limit], 'REPORTING_COUNTRY_ROLLUPS');

        return result.map(row => {
          const impressions = parseInt(row.impressions);
          const revenue = parseFloat(row.revenue);
          return {
            countryCode: row.country_code,
            revenue,
            impressions,
            ecpm: impressions > 0 ? (revenue / impressions) * 1000 : 0,
          };
        });
      }

      const sql = `
        SELECT
          r.country_code,
          SUM(r.revenue_usd) AS revenue,
          COUNT(DISTINCT r.impression_id) AS impressions
        FROM analytics_revenue_events r
        WHERE r.publisher_id = $1::uuid
          AND r.observed_at >= $2
          AND r.observed_at < $3
          AND r.is_test_mode = false
        GROUP BY r.country_code
        ORDER BY revenue DESC
        LIMIT $4
      `;

      const result = await runQuery<{
        country_code: string;
        revenue: string;
        impressions: string;
      }>(sql, [publisherId, startDate, endDate, limit]);

      return result.map(row => ({
        countryCode: row.country_code,
        revenue: parseFloat(row.revenue),
        impressions: parseInt(row.impressions),
        ecpm: parseInt(row.impressions) > 0
          ? (parseFloat(row.revenue) / parseInt(row.impressions)) * 1000
          : 0,
      }));
    } catch (error) {
      logger.error('Failed to get country breakdown', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get top performing apps
   */
  async getTopApps(
    publisherId: string,
    startDate: Date,
    endDate: Date,
    limit: number = 10
  ): Promise<Array<{
    appId: string;
    revenue: number;
    impressions: number;
    ecpm: number;
  }>> {
    try {
      if (await rollupTableExists('analytics_metrics_rollups')) {
        const sql = `
          SELECT
            app_id,
            COALESCE(SUM(total_revenue), 0) AS revenue,
            COALESCE(SUM(impression_count), 0) AS impressions
          FROM analytics_metrics_rollups
          WHERE publisher_id = $1::text
            AND bucket_date >= $2::date
            AND bucket_date < $3::date
            AND app_id <> ''
          GROUP BY app_id
          ORDER BY revenue DESC
          LIMIT $4
        `;

        const result = await runQuery<{
          app_id: string;
          revenue: string;
          impressions: string;
        }>(sql, [publisherId, formatUtcDate(startDate), formatUtcDate(endDate), limit], 'REPORTING_TOP_APPS_ROLLUPS');

        return result.map(row => {
          const impressions = parseInt(row.impressions);
          const revenue = parseFloat(row.revenue);
          return {
            appId: row.app_id,
            revenue,
            impressions,
            ecpm: impressions > 0 ? (revenue / impressions) * 1000 : 0,
          };
        });
      }

      const sql = `
        SELECT
          r.app_id,
          SUM(r.revenue_usd) AS revenue,
          COUNT(DISTINCT r.impression_id) AS impressions
        FROM analytics_revenue_events r
        WHERE r.publisher_id = $1::uuid
          AND r.observed_at >= $2
          AND r.observed_at < $3
          AND r.is_test_mode = false
        GROUP BY r.app_id
        ORDER BY revenue DESC
        LIMIT $4
      `;

      const result = await runQuery<{
        app_id: string;
        revenue: string;
        impressions: string;
      }>(sql, [publisherId, startDate, endDate, limit]);

      return result.map(row => ({
        appId: row.app_id,
        revenue: parseFloat(row.revenue),
        impressions: parseInt(row.impressions),
        ecpm: parseInt(row.impressions) > 0
          ? (parseFloat(row.revenue) / parseInt(row.impressions)) * 1000
          : 0,
      }));
    } catch (error) {
      logger.error('Failed to get top apps', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get real-time stats (last hour)
   */
  async getRealtimeStats(publisherId: string): Promise<{
    lastHourRevenue: number;
    lastHourImpressions: number;
    activeAdapters: number;
  }> {
    try {
      const oneHourAgo = new Date(Date.now() - 60 * 60 * 1000);
      
      const sql = `
        SELECT
          COALESCE(SUM(r.revenue_usd), 0) AS revenue,
          COALESCE(COUNT(DISTINCT r.impression_id), 0) AS impressions,
          COALESCE(COUNT(DISTINCT r.adapter_id), 0) AS active_adapters
        FROM analytics_revenue_events r
        WHERE r.publisher_id = $1::uuid
          AND r.observed_at >= $2
          AND r.is_test_mode = false
      `;

      const result = await runQuery<{
        revenue: string;
        impressions: string;
        active_adapters: string;
      }>(sql, [publisherId, oneHourAgo]);

      if (result.length === 0) {
        return {
          lastHourRevenue: 0,
          lastHourImpressions: 0,
          activeAdapters: 0,
        };
      }

      const row = result[0];
      return {
        lastHourRevenue: parseFloat(row.revenue),
        lastHourImpressions: parseInt(row.impressions),
        activeAdapters: parseInt(row.active_adapters),
      };
    } catch (error) {
      logger.error('Failed to get realtime stats', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get adapter health scores with real-time monitoring
   */
  async getAdapterHealthScores(publisherId: string): Promise<AdapterHealthScore[]> {
    try {
      const last24h = new Date(Date.now() - 24 * 60 * 60 * 1000);
      
      const sql = `
        WITH adapter_stats AS (
          SELECT
            adapter_id,
            COALESCE(adapter_name, 'unknown') AS adapter_name,
            COUNT(*) AS total_requests,
            COUNT(*) FILTER (WHERE status = 'success' OR filled IS TRUE) AS success_count,
            COUNT(*) FILTER (WHERE status = 'error') AS error_count,
            AVG(latency_ms) AS avg_latency,
            100.0 * COUNT(*) FILTER (WHERE filled IS TRUE) / NULLIF(COUNT(*), 0) AS fill_rate,
            SUM(revenue_usd) AS revenue
          FROM analytics_impressions
          WHERE publisher_id = $1::uuid
            AND observed_at >= $2
          GROUP BY adapter_id, adapter_name
        ),
        total_revenue AS (
          SELECT COALESCE(SUM(revenue), 0) AS total FROM adapter_stats
        ),
        adapter_errors AS (
          SELECT
            adapter_id,
            COUNT(*) AS error_count_24h,
            MAX(message) FILTER (WHERE event_type = 'error') AS last_issue
          FROM analytics_sdk_telemetry
          WHERE publisher_id = $1::uuid
            AND observed_at >= $2
          GROUP BY adapter_id
        )
        SELECT
          s.adapter_id,
          s.adapter_name,
          COALESCE(100.0 * s.success_count / NULLIF(s.total_requests, 0), 0) AS uptime,
          COALESCE(100.0 * s.error_count / NULLIF(s.total_requests, 0), 0) AS error_rate,
          COALESCE(s.avg_latency, 0) AS avg_latency,
          COALESCE(s.fill_rate, 0) AS fill_rate,
          CASE WHEN t.total = 0 THEN 0 ELSE 100.0 * s.revenue / t.total END AS revenue_share,
          COALESCE(e.error_count_24h, 0) AS issues_last_24h,
          e.last_issue
        FROM adapter_stats s
        CROSS JOIN total_revenue t
        LEFT JOIN adapter_errors e ON s.adapter_id = e.adapter_id
        ORDER BY revenue_share DESC
      `;

      const result = await runQuery<{
        adapter_id: string;
        adapter_name: string;
        uptime: string;
        error_rate: string;
        avg_latency: string | null;
        fill_rate: string | null;
        revenue_share: string;
        issues_last_24h: string;
        last_issue: string | null;
      }>(sql, [publisherId, last24h]);

      return result.map(row => {
        const uptime = parseFloat(row.uptime);
        const errorRate = parseFloat(row.error_rate);
        const fillRate = row.fill_rate ? parseFloat(row.fill_rate) : 0;
        const avgLatency = row.avg_latency ? parseFloat(row.avg_latency) : 0;
        
        // Calculate health score (0-100)
        let healthScore = 100;
        healthScore -= (100 - uptime) * 0.4; // Uptime weighted 40%
        healthScore -= errorRate * 30; // Error rate weighted 30%
        healthScore -= Math.max(0, (avgLatency - 500) / 10); // Latency penalty after 500ms
        healthScore -= (100 - fillRate) * 0.3; // Fill rate weighted 30%
        healthScore = Math.max(0, Math.min(100, healthScore));

        // Determine status
        let status: 'healthy' | 'degraded' | 'critical' | 'offline';
        if (uptime < 50) status = 'offline';
        else if (healthScore < 50) status = 'critical';
        else if (healthScore < 70) status = 'degraded';
        else status = 'healthy';

        return {
          adapterId: row.adapter_id,
          adapterName: row.adapter_name,
          healthScore: Math.round(healthScore),
          uptime,
          errorRate,
          avgResponseTime: avgLatency,
          fillRate,
          revenueShare: parseFloat(row.revenue_share),
          status,
          lastIssue: row.last_issue || undefined,
          issuesLast24h: parseInt(row.issues_last_24h),
        };
      });
    } catch (error) {
      logger.error('Failed to get adapter health scores', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get fraud detection metrics
   */
  async getFraudMetrics(
    publisherId: string,
    startDate: Date,
    endDate: Date
  ): Promise<FraudMetrics> {
    try {
      const sql = `
        SELECT
          COUNT(*) AS total_requests,
          COUNT(*) FILTER (WHERE blocked) AS fraud_requests,
          100.0 * COUNT(*) FILTER (WHERE blocked) / NULLIF(COUNT(*), 0) AS fraud_rate,
          COUNT(*) FILTER (WHERE fraud_type = 'givt') AS givt_detections,
          COUNT(*) FILTER (WHERE fraud_type = 'sivt') AS sivt_detections,
          COUNT(*) FILTER (WHERE fraud_type = 'ml_fraud') AS ml_detections,
          COUNT(*) FILTER (WHERE fraud_type = 'anomaly') AS anomaly_detections,
          COALESCE(SUM(revenue_blocked_cents), 0) / 100.0 AS blocked_revenue
        FROM analytics_fraud_events
        WHERE publisher_id = $1::text
          AND observed_at >= $2
          AND observed_at < $3
      `;

      const result = await runQuery<{
        total_requests: string;
        fraud_requests: string;
        fraud_rate: string;
        givt_detections: string;
        sivt_detections: string;
        ml_detections: string;
        anomaly_detections: string;
        blocked_revenue: string;
      }>(sql, [publisherId, startDate, endDate]);

      const typesSql = `
        SELECT
          fraud_type,
          COUNT(*) AS count
        FROM analytics_fraud_events
        WHERE publisher_id = $1::text
          AND observed_at >= $2
          AND observed_at < $3
          AND blocked
        GROUP BY fraud_type
        ORDER BY count DESC
        LIMIT 10
      `;

      const typesResult = await runQuery<{
        fraud_type: string;
        count: string;
      }>(typesSql, [publisherId, startDate, endDate]);

      if (result.length === 0) {
        return {
          totalRequests: 0,
          fraudRequests: 0,
          fraudRate: 0,
          givtDetections: 0,
          sivtDetections: 0,
          mlDetections: 0,
          anomalyDetections: 0,
          blockedRevenue: 0,
          topFraudTypes: [],
        };
      }

      const row = result[0];
      return {
        totalRequests: parseInt(row.total_requests),
        fraudRequests: parseInt(row.fraud_requests),
        fraudRate: parseFloat(row.fraud_rate),
        givtDetections: parseInt(row.givt_detections),
        sivtDetections: parseInt(row.sivt_detections),
        mlDetections: parseInt(row.ml_detections),
        anomalyDetections: parseInt(row.anomaly_detections),
        blockedRevenue: parseFloat(row.blocked_revenue),
        topFraudTypes: typesResult.map(t => ({
          type: t.fraud_type,
          count: parseInt(t.count),
        })),
      };
    } catch (error) {
      logger.error('Failed to get fraud metrics', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get quality metrics (viewability, completion, brand safety)
   */
  async getQualityMetrics(
    publisherId: string,
    startDate: Date,
    endDate: Date
  ): Promise<QualityMetrics> {
    try {
      const sql = `
        WITH impression_quality AS (
          SELECT
            COUNT(*) AS total_impressions,
            COUNT(*) FILTER (WHERE viewable) AS viewable_impressions,
            COUNT(*) FILTER (WHERE measurable) AS measurable_impressions
          FROM analytics_impressions
          WHERE publisher_id = $1::uuid
            AND observed_at >= $2
            AND observed_at < $3
        ),
        click_metrics AS (
          SELECT COUNT(*) AS clicks
          FROM analytics_clicks
          WHERE publisher_id = $1::text
            AND observed_at >= $2
            AND observed_at < $3
        ),
        fraud_metrics AS (
          SELECT
            COUNT(*) AS total_events,
            COUNT(*) FILTER (WHERE blocked) AS blocked_events
          FROM analytics_fraud_events
          WHERE publisher_id = $1::text
            AND observed_at >= $2
            AND observed_at < $3
        ),
        brand_safety AS (
          SELECT AVG(risk_score) AS avg_risk_score
          FROM analytics_creative_scans
          WHERE publisher_id = $1::text
            AND observed_at >= $2
            AND observed_at < $3
        ),
        telemetry AS (
          SELECT
            COUNT(*) AS total_events,
            COUNT(*) FILTER (WHERE event_type = 'anr') AS anr_events,
            COUNT(*) FILTER (WHERE event_type = 'crash') AS crash_events
          FROM analytics_sdk_telemetry
          WHERE publisher_id = $1::uuid
            AND observed_at >= $2
            AND observed_at < $3
        )
        SELECT
          COALESCE(100.0 * iq.viewable_impressions / NULLIF(iq.total_impressions, 0), 0) AS viewability_rate,
          0::double precision AS completion_rate,
          COALESCE(100.0 * cm.clicks / NULLIF(iq.total_impressions, 0), 0) AS ctr,
          COALESCE(100.0 * fm.blocked_events / NULLIF(iq.total_impressions, 0), 0) AS ivt_rate,
          COALESCE(100 - bs.avg_risk_score, 100) AS brand_safety,
          100::double precision AS user_experience,
          COALESCE(100.0 * t.anr_events / NULLIF(t.total_events, 0), 0) AS anr_rate,
          COALESCE(100.0 * t.crash_events / NULLIF(t.total_events, 0), 0) AS crash_rate
        FROM impression_quality iq
        LEFT JOIN click_metrics cm ON TRUE
        LEFT JOIN fraud_metrics fm ON TRUE
        LEFT JOIN brand_safety bs ON TRUE
        LEFT JOIN telemetry t ON TRUE
      `;

      const result = await runQuery<{
        viewability_rate: string;
        completion_rate: string;
        ctr: string;
        ivt_rate: string;
        brand_safety: string;
        user_experience: string;
        anr_rate: string;
        crash_rate: string;
      }>(sql, [publisherId, startDate, endDate]);

      if (result.length === 0) {
        return {
          viewabilityRate: 0,
          completionRate: 0,
          clickThroughRate: 0,
          invalidTrafficRate: 0,
          brandSafetyScore: 100,
          userExperienceScore: 100,
          anrRate: 0,
          crashRate: 0,
        };
      }

      const row = result[0];
      return {
        viewabilityRate: parseFloat(row.viewability_rate),
        completionRate: parseFloat(row.completion_rate),
        clickThroughRate: parseFloat(row.ctr),
        invalidTrafficRate: parseFloat(row.ivt_rate),
        brandSafetyScore: parseFloat(row.brand_safety),
        userExperienceScore: parseFloat(row.user_experience),
        anrRate: parseFloat(row.anr_rate),
        crashRate: parseFloat(row.crash_rate),
      };
    } catch (error) {
      logger.error('Failed to get quality metrics', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get revenue projections using simple linear regression
   */
  async getRevenueProjections(
    publisherId: string,
    days: number = 7
  ): Promise<RevenueProjection[]> {
    try {
      // Get last 30 days of data for trend analysis
      const lookbackDays = 30;
      const startDate = new Date(Date.now() - lookbackDays * 24 * 60 * 60 * 1000);
      const endDate = new Date();

      const result = await (async () => {
        if (await rollupTableExists('analytics_metrics_rollups')) {
          const sql = `
            SELECT
              bucket_date AS bucket,
              COALESCE(SUM(total_revenue), 0) AS revenue
            FROM analytics_metrics_rollups
            WHERE publisher_id = $1
              AND bucket_date >= $2::date
              AND bucket_date < $3::date
            GROUP BY bucket
            ORDER BY bucket ASC
          `;
          return runQuery<{ bucket: string; revenue: string }>(
            sql,
            [publisherId, formatUtcDate(startDate), formatUtcDate(endDate)],
            'REPORTING_PROJECTIONS_ROLLUPS'
          );
        }

        const sql = `
          SELECT
            date_trunc('day', observed_at) AS bucket,
            SUM(revenue_usd) AS revenue
          FROM analytics_revenue_events
          WHERE publisher_id = $1
            AND observed_at >= $2
            AND observed_at < $3
            AND is_test_mode = false
          GROUP BY bucket
          ORDER BY bucket ASC
        `;

        return runQuery<{ bucket: Date; revenue: string }>(sql, [publisherId, startDate, endDate]);
      })();

      if (result.length < 7) {
        // Not enough data for projection
        return [];
      }

      // Simple linear regression
      const dataPoints = result.map((r: any, i) => ({
        x: i,
        y: parseFloat(r.revenue),
      }));

      const n = dataPoints.length;
      const sumX = dataPoints.reduce((acc, p) => acc + p.x, 0);
      const sumY = dataPoints.reduce((acc, p) => acc + p.y, 0);
      const sumXY = dataPoints.reduce((acc, p) => acc + p.x * p.y, 0);
      const sumXX = dataPoints.reduce((acc, p) => acc + p.x * p.x, 0);

      const slope = (n * sumXY - sumX * sumY) / (n * sumXX - sumX * sumX);
      const intercept = (sumY - slope * sumX) / n;

      // Calculate standard error
      const predictions = dataPoints.map(p => slope * p.x + intercept);
      const residuals = dataPoints.map((p, i) => p.y - predictions[i]);
      const mse = residuals.reduce((acc, r) => acc + r * r, 0) / n;
      const stdError = Math.sqrt(mse);

      // Generate projections
      const projections: RevenueProjection[] = [];
      for (let i = 1; i <= days; i++) {
        const x = n + i;
        const projected = slope * x + intercept;
        const margin = 1.96 * stdError * Math.sqrt(1 + 1/n + Math.pow(x - sumX/n, 2) / (sumXX - sumX*sumX/n));

        const projectionDate = new Date(endDate);
        projectionDate.setDate(projectionDate.getDate() + i);

        projections.push({
          date: projectionDate.toISOString().split('T')[0],
          projectedRevenue: Math.max(0, projected),
          lowerBound: Math.max(0, projected - margin),
          upperBound: projected + margin,
          confidence: 0.95,
        });
      }

      return projections;
    } catch (error) {
      logger.error('Failed to get revenue projections', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get cohort analysis for user retention and LTV
   */
  async getCohortAnalysis(
    publisherId: string,
    startDate: Date,
    endDate: Date
  ): Promise<CohortMetrics[]> {
    try {
      const sql = `
        WITH user_first_impressions AS (
          SELECT
            user_id,
            MIN(observed_at)::date AS cohort_date
          FROM analytics_impressions
          WHERE publisher_id = $1
            AND observed_at >= $2
            AND observed_at < $3
            AND user_id IS NOT NULL
          GROUP BY user_id
        ),
        revenue_by_user AS (
          SELECT
            i.user_id,
            u.cohort_date,
            DATE_PART('day', r.observed_at::date - u.cohort_date) AS days_since_cohort,
            SUM(r.revenue_usd) AS revenue
          FROM analytics_revenue_events r
          INNER JOIN analytics_impressions i ON r.impression_id = i.event_id
          INNER JOIN user_first_impressions u ON u.user_id = i.user_id
          WHERE r.publisher_id = $1
          GROUP BY i.user_id, u.cohort_date, days_since_cohort
        ),
        cohort_stats AS (
          SELECT
            cohort_date,
            COUNT(DISTINCT user_id) AS user_count,
            SUM(CASE WHEN days_since_cohort = 0 THEN revenue ELSE 0 END) AS day0_revenue,
            SUM(CASE WHEN days_since_cohort = 1 THEN revenue ELSE 0 END) AS day1_revenue,
            SUM(CASE WHEN days_since_cohort BETWEEN 0 AND 7 THEN revenue ELSE 0 END) AS day7_revenue,
            SUM(CASE WHEN days_since_cohort BETWEEN 0 AND 30 THEN revenue ELSE 0 END) AS day30_revenue,
            COUNT(DISTINCT CASE WHEN days_since_cohort >= 1 THEN user_id END) * 100.0 / NULLIF(COUNT(DISTINCT user_id), 0) AS retention_day1,
            COUNT(DISTINCT CASE WHEN days_since_cohort >= 7 THEN user_id END) * 100.0 / NULLIF(COUNT(DISTINCT user_id), 0) AS retention_day7,
            COUNT(DISTINCT CASE WHEN days_since_cohort >= 30 THEN user_id END) * 100.0 / NULLIF(COUNT(DISTINCT user_id), 0) AS retention_day30
          FROM revenue_by_user
          GROUP BY cohort_date
        )
        SELECT
          cohort_date,
          user_count,
          day0_revenue,
          day1_revenue,
          day7_revenue,
          day30_revenue,
          CASE WHEN user_count = 0 THEN 0 ELSE day30_revenue / user_count END AS ltv,
          retention_day1,
          retention_day7,
          retention_day30,
          CASE WHEN user_count = 0 THEN 0 ELSE day30_revenue / user_count END AS arpu
        FROM cohort_stats
        WHERE user_count >= 100
        ORDER BY cohort_date DESC
      `;

      const result = await runQuery<{
        cohort_date: Date;
        user_count: string;
        day0_revenue: string;
        day1_revenue: string;
        day7_revenue: string;
        day30_revenue: string;
        ltv: string;
        retention_day1: string;
        retention_day7: string;
        retention_day30: string;
        arpu: string;
      }>(sql, [publisherId, startDate, endDate]);

      return result.map(row => ({
        cohortDate: new Date(row.cohort_date).toISOString().split('T')[0],
        userCount: parseInt(row.user_count),
        day0Revenue: parseFloat(row.day0_revenue),
        day1Revenue: parseFloat(row.day1_revenue),
        day7Revenue: parseFloat(row.day7_revenue),
        day30Revenue: parseFloat(row.day30_revenue),
        ltv: parseFloat(row.ltv),
        retentionDay1: parseFloat(row.retention_day1),
        retentionDay7: parseFloat(row.retention_day7),
        retentionDay30: parseFloat(row.retention_day30),
        arpu: parseFloat(row.arpu),
      }));
    } catch (error) {
      logger.error('Failed to get cohort analysis', { publisherId, error });
      throw error;
    }
  }

  /**
   * Get anomaly detection alerts using statistical methods
   */
  async getAnomalies(
    publisherId: string,
    hours: number = 24
  ): Promise<AnomalyAlert[]> {
    try {
      const startTime = new Date(Date.now() - hours * 60 * 60 * 1000);
      const lookbackTime = new Date(Date.now() - (hours + 168) * 60 * 60 * 1000); // Additional 7 days for baseline

      const sql = `
        WITH hourly_metrics AS (
          SELECT
            date_trunc('hour', observed_at) AS hour,
            SUM(revenue_usd) AS revenue,
            COUNT(*) AS requests,
            100.0 * COUNT(*) FILTER (WHERE status = 'error') / NULLIF(COUNT(*), 0) AS error_rate,
            AVG(latency_ms) AS avg_latency
          FROM analytics_impressions
          WHERE publisher_id = $1
            AND observed_at >= $2
          GROUP BY hour
        ),
        baseline_stats AS (
          SELECT
            AVG(revenue) AS mean_revenue,
            STDDEV_POP(revenue) AS std_revenue,
            AVG(requests) AS mean_requests,
            STDDEV_POP(requests) AS std_requests,
            AVG(error_rate) AS mean_error_rate,
            STDDEV_POP(error_rate) AS std_error_rate,
            AVG(avg_latency) AS mean_latency,
            STDDEV_POP(avg_latency) AS std_latency
          FROM hourly_metrics
          WHERE hour < $3
        ),
        recent_metrics AS (
          SELECT * FROM hourly_metrics WHERE hour >= $3
        )
        SELECT
          r.hour,
          r.revenue,
          r.requests,
          r.error_rate,
          r.avg_latency,
          b.mean_revenue,
          b.std_revenue,
          b.mean_requests,
          b.std_requests,
          b.mean_error_rate,
          b.std_error_rate,
          b.mean_latency,
          b.std_latency,
          ABS(r.revenue - b.mean_revenue) / NULLIF(b.std_revenue, 0) AS revenue_z_score,
          ABS(r.requests - b.mean_requests) / NULLIF(b.std_requests, 0) AS requests_z_score,
          ABS(r.error_rate - b.mean_error_rate) / NULLIF(b.std_error_rate, 0) AS error_z_score,
          ABS(r.avg_latency - b.mean_latency) / NULLIF(b.std_latency, 0) AS latency_z_score
        FROM recent_metrics r
        CROSS JOIN baseline_stats b
        WHERE (ABS(r.revenue - b.mean_revenue) / NULLIF(b.std_revenue, 0)) > 2.5
           OR (ABS(r.requests - b.mean_requests) / NULLIF(b.std_requests, 0)) > 2.5
           OR (ABS(r.error_rate - b.mean_error_rate) / NULLIF(b.std_error_rate, 0)) > 3
           OR (ABS(r.avg_latency - b.mean_latency) / NULLIF(b.std_latency, 0)) > 3
        ORDER BY r.hour DESC
      `;

      const result = await runQuery<{
        hour: Date;
        revenue: string;
        requests: string;
        error_rate: string;
        avg_latency: string;
        mean_revenue: string;
        std_revenue: string | null;
        mean_requests: string;
        std_requests: string | null;
        mean_error_rate: string;
        std_error_rate: string | null;
        mean_latency: string;
        std_latency: string | null;
        revenue_z_score: string | null;
        requests_z_score: string | null;
        error_z_score: string | null;
        latency_z_score: string | null;
      }>(sql, [publisherId, lookbackTime, startTime]);

      const anomalies: AnomalyAlert[] = [];

      result.forEach((row, index) => {
        const hourIso = new Date(row.hour).toISOString();
        const revenueZScore = row.revenue_z_score ? parseFloat(row.revenue_z_score) : 0;
        const requestsZScore = row.requests_z_score ? parseFloat(row.requests_z_score) : 0;
        const errorZScore = row.error_z_score ? parseFloat(row.error_z_score) : 0;
        const latencyZScore = row.latency_z_score ? parseFloat(row.latency_z_score) : 0;

        // Revenue anomaly
        if (revenueZScore > 2.5) {
          const currentRevenue = parseFloat(row.revenue);
          const expectedRevenue = parseFloat(row.mean_revenue);
          const deviation = ((currentRevenue - expectedRevenue) / expectedRevenue) * 100;

          anomalies.push({
            id: `revenue_${hourIso}_${index}`,
            timestamp: hourIso,
            type: deviation < 0 ? 'revenue_drop' : 'traffic_anomaly',
            severity: revenueZScore > 4 ? 'critical' : revenueZScore > 3 ? 'high' : 'medium',
            metric: 'revenue',
            currentValue: currentRevenue,
            expectedValue: expectedRevenue,
            deviation,
            description: `Revenue ${deviation > 0 ? 'spike' : 'drop'} of ${Math.abs(deviation).toFixed(1)}% (${revenueZScore.toFixed(2)} σ)`,
          });
        }

        // Traffic anomaly
        if (requestsZScore > 2.5) {
          const currentRequests = parseFloat(row.requests);
          const expectedRequests = parseFloat(row.mean_requests);
          const deviation = ((currentRequests - expectedRequests) / expectedRequests) * 100;

          anomalies.push({
            id: `traffic_${hourIso}_${index}`,
            timestamp: hourIso,
            type: 'traffic_anomaly',
            severity: requestsZScore > 4 ? 'critical' : requestsZScore > 3 ? 'high' : 'medium',
            metric: 'requests',
            currentValue: currentRequests,
            expectedValue: expectedRequests,
            deviation,
            description: `Traffic ${deviation > 0 ? 'surge' : 'drop'} of ${Math.abs(deviation).toFixed(1)}% (${requestsZScore.toFixed(2)} σ)`,
          });
        }

        // Error rate anomaly
        if (errorZScore > 3) {
          const currentErrorRate = parseFloat(row.error_rate);
          const expectedErrorRate = parseFloat(row.mean_error_rate);
          const deviation = currentErrorRate - expectedErrorRate;

          anomalies.push({
            id: `error_${hourIso}_${index}`,
            timestamp: hourIso,
            type: 'performance_degradation',
            severity: errorZScore > 5 ? 'critical' : errorZScore > 4 ? 'high' : 'medium',
            metric: 'error_rate',
            currentValue: currentErrorRate,
            expectedValue: expectedErrorRate,
            deviation,
            description: `Error rate spike: ${currentErrorRate.toFixed(2)}% vs expected ${expectedErrorRate.toFixed(2)}% (${errorZScore.toFixed(2)} σ)`,
          });
        }

        // Latency anomaly
        if (latencyZScore > 3) {
          const currentLatency = parseFloat(row.avg_latency);
          const expectedLatency = parseFloat(row.mean_latency);
          const deviation = ((currentLatency - expectedLatency) / expectedLatency) * 100;

          anomalies.push({
            id: `latency_${hourIso}_${index}`,
            timestamp: hourIso,
            type: 'performance_degradation',
            severity: latencyZScore > 5 ? 'critical' : latencyZScore > 4 ? 'high' : 'medium',
            metric: 'latency',
            currentValue: currentLatency,
            expectedValue: expectedLatency,
            deviation,
            description: `Latency spike: ${currentLatency.toFixed(0)}ms vs expected ${expectedLatency.toFixed(0)}ms (${latencyZScore.toFixed(2)} σ)`,
          });
        }
      });

      // Check for fraud spikes
      const fraudSql = `
        SELECT
          date_trunc('hour', observed_at) AS hour,
          COUNT(*) FILTER (WHERE blocked) AS fraud_count,
          COUNT(*) AS total_count,
          100.0 * COUNT(*) FILTER (WHERE blocked) / NULLIF(COUNT(*), 0) AS fraud_rate
        FROM analytics_fraud_events
        WHERE publisher_id = $1
          AND observed_at >= $2
        GROUP BY hour
        HAVING (100.0 * COUNT(*) FILTER (WHERE blocked) / NULLIF(COUNT(*), 0)) > 5
        ORDER BY hour DESC
      `;

      const fraudResult = await runQuery<{
        hour: Date;
        fraud_count: string;
        total_count: string;
        fraud_rate: string;
      }>(fraudSql, [publisherId, startTime]);

      fraudResult.forEach((row, index) => {
        const fraudRate = parseFloat(row.fraud_rate);
        const hourIso = new Date(row.hour).toISOString();
        anomalies.push({
          id: `fraud_${hourIso}_${index}`,
          timestamp: hourIso,
          type: 'fraud_spike',
          severity: fraudRate > 15 ? 'critical' : fraudRate > 10 ? 'high' : 'medium',
          metric: 'fraud_rate',
          currentValue: fraudRate,
          expectedValue: 2.0, // Baseline fraud rate
          deviation: fraudRate - 2.0,
          description: `Elevated fraud detection: ${fraudRate.toFixed(2)}% of traffic (${row.fraud_count} blocked requests)`,
        });
      });

      return anomalies;
    } catch (error) {
      logger.error('Failed to get anomalies', { publisherId, error });
      throw error;
    }
  }
}

export const reportingService = new ReportingServiceImpl();

export default reportingService;
