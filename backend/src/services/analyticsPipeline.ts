import { subDays, addDays, addMonths } from 'date-fns';
import type { QueryResultRow } from 'pg';
import reportingService from './reportingService';
import { query as pgQuery } from '../utils/postgres';

const runQuery = async <T extends QueryResultRow = QueryResultRow>(
  sql: string,
  params: ReadonlyArray<unknown>,
  label?: string
): Promise<T[]> => {
  const queryLabel = label ?? 'ANALYTICS_PIPELINE';
  const { rows } = await pgQuery<T>(sql, params, { replica: true, label: queryLabel });
  return rows;
};

type AnalyticsOverview = {
  revenue: {
    today: number;
    yesterday: number;
    thisMonth: number;
    lastMonth: number;
    lifetime: number;
  };
  impressions: number;
  clicks: number;
  ecpm: number;
  ctr: number;
};

type ImpressionsBreakdownRow =
  | { groupBy: 'date'; date: string; impressions: number }
  | { groupBy: 'placement'; placementId: string; placementName: string; impressions: number }
  | { groupBy: 'adapter'; adapterId: string; adapterName: string; impressions: number };

function startOfUtcDay(d: Date): Date {
  return new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));
}

function startOfUtcMonth(d: Date): Date {
  return new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1));
}

export const getAnalyticsOverview = async (publisherId: string): Promise<AnalyticsOverview> => {
  const now = new Date();
  const todayStart = startOfUtcDay(now);
  const tomorrowStart = addDays(todayStart, 1);
  const yesterdayStart = subDays(todayStart, 1);

  const thisMonthStart = startOfUtcMonth(now);
  const nextMonthStart = addMonths(thisMonthStart, 1);
  const lastMonthStart = subMonthsUtc(thisMonthStart, 1);

  // Compute revenue windows using the same logic as the Revenue endpoints.
  const [todayStats, yesterdayStats, thisMonthStats, lastMonthStats, lifetimeStats] = await Promise.all([
    reportingService.getRevenueStats(publisherId, todayStart, tomorrowStart),
    reportingService.getRevenueStats(publisherId, yesterdayStart, todayStart),
    reportingService.getRevenueStats(publisherId, thisMonthStart, nextMonthStart),
    reportingService.getRevenueStats(publisherId, lastMonthStart, thisMonthStart),
    reportingService.getRevenueStats(publisherId, new Date(Date.UTC(1970, 0, 1)), tomorrowStart),
  ]);

  return {
    revenue: {
      today: todayStats.totalRevenue,
      yesterday: yesterdayStats.totalRevenue,
      thisMonth: thisMonthStats.totalRevenue,
      lastMonth: lastMonthStats.totalRevenue,
      lifetime: lifetimeStats.totalRevenue,
    },
    impressions: lifetimeStats.totalImpressions,
    clicks: lifetimeStats.totalClicks,
    ecpm: Number((lifetimeStats.ecpm ?? 0).toFixed(2)),
    ctr: Number((lifetimeStats.ctr ?? 0).toFixed(2)),
  };
};

function subMonthsUtc(d: Date, months: number): Date {
  const year = d.getUTCFullYear();
  const month = d.getUTCMonth();
  return new Date(Date.UTC(year, month - months, 1));
}

export const getAnalyticsTimeSeries = async (
  publisherId: string,
  metric: 'revenue' | 'impressions' | 'clicks',
  startDate?: string,
  endDate?: string,
  granularity: 'hour' | 'day' = 'day'
) => {
  const now = new Date();
  const end = endDate ? new Date(endDate) : now;
  const start = startDate ? new Date(startDate) : subDays(end, 30);

  // Validate dates and cap range to 31 days (FIX-11: 673)
  const endSafe = Number.isNaN(end.getTime()) ? now : end;
  const startSafe = Number.isNaN(start.getTime()) ? subDays(endSafe, 30) : start;
  const maxRangeMs = 31 * 24 * 60 * 60 * 1000;
  const rangeMs = endSafe.getTime() - startSafe.getTime();
  const startCapped = rangeMs > maxRangeMs ? subDays(endSafe, 31) : startSafe;

  const points = await reportingService.getTimeSeriesData(publisherId, startCapped, endSafe, granularity);

  return points.map((p) => {
    const date = granularity === 'hour' ? p.timestamp : p.timestamp.split('T')[0];
    const value =
      metric === 'revenue'
        ? Number((p.revenue ?? 0).toFixed(2))
        : metric === 'impressions'
          ? p.impressions ?? 0
          : p.clicks ?? 0;

    return {
      date,
      value,
      revenue: p.revenue ?? 0,
      impressions: p.impressions ?? 0,
      clicks: p.clicks ?? 0,
    };
  });
};

export const getPerformanceBreakdown = async (
  publisherId: string,
  groupBy: 'placement' | 'adapter',
  startDate?: string,
  endDate?: string
) => {
  const now = new Date();
  const end = endDate ? new Date(endDate) : now;
  const start = startDate ? new Date(startDate) : subDays(end, 30);

  const endSafe = Number.isNaN(end.getTime()) ? now : end;
  const startSafe = Number.isNaN(start.getTime()) ? subDays(endSafe, 30) : start;

  if (groupBy === 'adapter') {
    const rows = await reportingService.getAdapterPerformance(publisherId, startSafe, endSafe);
    return rows.map((r) => ({
      id: r.adapterId,
      name: r.adapterName,
      revenue: r.revenue,
      impressions: r.impressions,
      ecpm: Number((r.ecpm ?? 0).toFixed(2)),
    }));
  }

  // Placement breakdown isn't exposed in reportingService yet; implement a minimal query here.
  const result = await runQuery<{
    placement_id: string;
    placement_name: string;
    revenue: string;
    impressions: string;
  }>(
    `
      SELECT
        r.placement_id::text AS placement_id,
        COALESCE(p.name, 'unknown') AS placement_name,
        COALESCE(SUM(r.revenue_usd), 0)::text AS revenue,
        COUNT(DISTINCT r.impression_id)::bigint::text AS impressions
      FROM analytics_revenue_events r
      LEFT JOIN placements p ON p.id = r.placement_id
      WHERE r.publisher_id = $1
        AND r.observed_at >= $2
        AND r.observed_at < $3
        AND r.is_test_mode = false
      GROUP BY r.placement_id, p.name
      ORDER BY COALESCE(SUM(r.revenue_usd), 0) DESC NULLS LAST
    `,
    [publisherId, startSafe, endSafe],
    'ANALYTICS_PERFORMANCE_PLACEMENT'
  );

  return result.map((row) => {
    const revenue = Number.parseFloat(row.revenue);
    const impressions = Number.parseInt(row.impressions, 10);
    const ecpm = impressions > 0 ? (revenue / impressions) * 1000 : 0;
    return {
      id: row.placement_id,
      name: row.placement_name,
      revenue: Number.isFinite(revenue) ? revenue : 0,
      impressions: Number.isFinite(impressions) ? impressions : 0,
      ecpm: Number((Number.isFinite(ecpm) ? ecpm : 0).toFixed(2)),
    };
  });
};

export const getImpressionsBreakdown = async (
  publisherId: string,
  groupBy: 'date' | 'placement' | 'adapter',
  startDate?: string,
  endDate?: string
): Promise<ImpressionsBreakdownRow[]> => {
  const now = new Date();
  const end = endDate ? new Date(endDate) : now;
  const start = startDate ? new Date(startDate) : subDays(end, 30);

  const endSafe = Number.isNaN(end.getTime()) ? now : end;
  const startSafe = Number.isNaN(start.getTime()) ? subDays(endSafe, 30) : start;

  if (groupBy === 'date') {
    const result = await runQuery<{ day: string; impressions: string }>(
      `
        SELECT
          to_char(date_trunc('day', observed_at), 'YYYY-MM-DD') AS day,
          COUNT(*)::bigint::text AS impressions
        FROM analytics_impressions
        WHERE publisher_id = $1
          AND observed_at >= $2
          AND observed_at < $3
          AND is_test_mode = false
        GROUP BY 1
        ORDER BY 1 ASC
      `,
      [publisherId, startSafe, endSafe],
      'ANALYTICS_IMPRESSIONS_BY_DAY'
    );

    return result.map((r) => ({
      groupBy: 'date',
      date: r.day,
      impressions: Number.parseInt(r.impressions, 10) || 0,
    }));
  }

  if (groupBy === 'adapter') {
    const result = await runQuery<{ adapter_id: string; adapter_name: string; impressions: string }>(
      `
        SELECT
          adapter_id::text AS adapter_id,
          COALESCE(adapter_name, 'unknown') AS adapter_name,
          COUNT(*)::bigint::text AS impressions
        FROM analytics_impressions
        WHERE publisher_id = $1
          AND observed_at >= $2
          AND observed_at < $3
          AND is_test_mode = false
        GROUP BY adapter_id, adapter_name
        ORDER BY COUNT(*) DESC
      `,
      [publisherId, startSafe, endSafe],
      'ANALYTICS_IMPRESSIONS_BY_ADAPTER'
    );

    return result.map((r) => ({
      groupBy: 'adapter',
      adapterId: r.adapter_id,
      adapterName: r.adapter_name,
      impressions: Number.parseInt(r.impressions, 10) || 0,
    }));
  }

  const result = await runQuery<{ placement_id: string; placement_name: string; impressions: string }>(
    `
      SELECT
        i.placement_id::text AS placement_id,
        COALESCE(p.name, 'unknown') AS placement_name,
        COUNT(*)::bigint::text AS impressions
      FROM analytics_impressions i
      LEFT JOIN placements p ON p.id = i.placement_id
      WHERE i.publisher_id = $1
        AND i.observed_at >= $2
        AND i.observed_at < $3
        AND i.is_test_mode = false
      GROUP BY i.placement_id, p.name
      ORDER BY COUNT(*) DESC
    `,
    [publisherId, startSafe, endSafe],
    'ANALYTICS_IMPRESSIONS_BY_PLACEMENT'
  );

  return result.map((r) => ({
    groupBy: 'placement',
    placementId: r.placement_id,
    placementName: r.placement_name,
    impressions: Number.parseInt(r.impressions, 10) || 0,
  }));
};
