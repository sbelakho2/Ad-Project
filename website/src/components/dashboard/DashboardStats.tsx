"use client";
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';

type ReportingOverview = {
  totalRevenue: number;
  totalImpressions: number;
  totalClicks: number;
  ecpm: number;
  ctr: number;
  fillRate: number;
};

type StatSnapshot = {
  revenue: number;
  impressions: number;
  ecpm: number;
  fillRate: number;
};

function percentChange(current: number, previous?: number): { label: string; trend: 'up' | 'down' | 'neutral' } {
  if (previous === undefined || previous === null) return { label: '—', trend: 'neutral' };
  if (previous === 0) return { label: current > 0 ? '+100%' : '0%', trend: current > 0 ? 'up' : 'neutral' };
  const delta = ((current - previous) / Math.abs(previous)) * 100;
  const rounded = Math.round(delta);
  const trend: 'up' | 'down' | 'neutral' = rounded > 0 ? 'up' : rounded < 0 ? 'down' : 'neutral';
  const sign = rounded > 0 ? '+' : '';
  return { label: `${sign}${rounded}%`, trend };
}

function SkeletonStat() {
  return (
    <div className="bg-white overflow-hidden shadow rounded-lg p-5 animate-pulse" aria-hidden="true">
      <div className="h-3 w-24 bg-gray-200 rounded mb-3" />
      <div className="h-8 w-28 bg-gray-200 rounded mb-2" />
      <div className="h-3 w-20 bg-gray-200 rounded" />
    </div>
  );
}

export default function DashboardStats() {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [kpis, setKpis] = useState<{ today: StatSnapshot; yesterday?: StatSnapshot } | null>(null);

  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const currencyFmt = useMemo(
    () => new Intl.NumberFormat(undefined, { style: 'currency', currency }),
    [currency]
  );
  const numberFmt = useMemo(() => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }), []);
  const pctFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'percent', maximumFractionDigits: 1 }), []);

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;

    const rangeFor = (key: 'today' | 'yesterday') => {
      const now = new Date();
      if (key === 'today') {
        const start = new Date(now);
        start.setHours(0, 0, 0, 0);
        return { start, end: now };
      }
      const start = new Date(now);
      start.setDate(now.getDate() - 1);
      start.setHours(0, 0, 0, 0);
      const end = new Date(start);
      end.setDate(start.getDate() + 1);
      return { start, end };
    };

    const fetchStats = async (key: 'today' | 'yesterday') => {
      const { start, end } = rangeFor(key);
      const params = new URLSearchParams({
        startDate: start.toISOString(),
        endDate: end.toISOString(),
      });
      const res = await api.get<ReportingOverview>(`/reporting/overview?${params.toString()}`, {
        signal: controller?.signal,
      });
      if (!res.success || !res.data) {
        throw new Error(res.error || `Failed to load ${key} KPIs`);
      }
      return {
        revenue: Math.max(0, res.data.totalRevenue ?? 0),
        impressions: Math.max(0, res.data.totalImpressions ?? 0),
        ecpm: Math.max(0, res.data.ecpm ?? 0),
        fillRate: Math.max(0, res.data.fillRate ?? 0),
      } satisfies StatSnapshot;
    };

    (async () => {
      const [todayStats, yesterdayStats] = await Promise.all([
        fetchStats('today'),
        fetchStats('yesterday').catch((err) => {
          console.warn('[DashboardStats] Failed to load yesterday KPIs', err);
          return undefined;
        }),
      ]);
      if (!alive) return;
      setKpis({ today: todayStats, yesterday: yesterdayStats });
      setLoading(false);
    })().catch((e) => {
      if (!alive) return;
      setError(e?.message || 'Network error');
      setLoading(false);
    });

    return () => {
      alive = false;
      controller?.abort();
    };
  }, []);

  if (loading) {
    return (
      <div className="mt-8 grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-4" aria-busy="true">
        <SkeletonStat />
        <SkeletonStat />
        <SkeletonStat />
        <SkeletonStat />
      </div>
    );
  }

  if (error) {
    return (
      <div role="alert" aria-live="polite" className="mt-8 rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
        {error}
      </div>
    );
  }

  const todayStats = kpis?.today ?? { revenue: 0, impressions: 0, ecpm: 0, fillRate: 0 };
  const yesterdayStats = kpis?.yesterday;

  const todayRevenue = todayStats.revenue;
  const yRev = yesterdayStats?.revenue;
  const impressions = todayStats.impressions;
  const yImp = yesterdayStats?.impressions;
  const ecpm = todayStats.ecpm;
  const yEcpm = yesterdayStats?.ecpm;
  const fill = Math.min(1, Math.max(0, todayStats.fillRate ?? 0));
  const yFill = Math.min(1, Math.max(0, yesterdayStats?.fillRate ?? 0));

  const revChange = percentChange(todayRevenue, yRev);
  const impChange = percentChange(impressions, yImp);
  const ecpmChange = percentChange(ecpm, yEcpm);
  const fillChange = percentChange(fill, yFill);

  return (
    <div className="mt-8 grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-4">
      <StatCard title="Today's Revenue" value={currencyFmt.format(todayRevenue)} change={revChange.label} trend={revChange.trend} />
      <StatCard title="Total Impressions" value={numberFmt.format(impressions)} change={impChange.label} trend={impChange.trend} />
      <StatCard title="eCPM" value={currencyFmt.format(ecpm)} change={ecpmChange.label} trend={ecpmChange.trend} />
      <StatCard title="Fill Rate" value={pctFmt.format(fill)} change={fillChange.label} trend={fillChange.trend} />
    </div>
  );
}

function StatCard({
  title,
  value,
  change,
  trend,
}: {
  title: string;
  value: string;
  change: string;
  trend: 'up' | 'down' | 'neutral';
}) {
  const trendColor = {
    up: 'text-success',
    down: 'text-danger',
    neutral: 'text-inkMuted',
  }[trend];

  return (
    <div className="bg-white overflow-hidden shadow rounded-lg">
      <div className="p-5">
        <div className="flex items-center">
          <div className="flex-1">
            <dt className="text-sm font-medium text-gray-500 truncate">{title}</dt>
            <dd className="mt-1 text-3xl font-semibold text-gray-900">{value}</dd>
          </div>
        </div>
        <div className="mt-4">
          <span className={`text-sm font-medium ${trendColor}`}>{change}</span>
          <span className="text-sm text-gray-500 ml-2">vs yesterday</span>
        </div>
      </div>
    </div>
  );
}
