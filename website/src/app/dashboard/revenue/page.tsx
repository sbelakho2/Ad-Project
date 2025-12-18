"use client";

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Revenue Page"
// Revenue dashboard wired to live APIs with localized charts and resilient states

import {
  ArrowTrendingUpIcon,
  BanknotesIcon,
  ChartBarIcon,
  CurrencyDollarIcon,
} from '@heroicons/react/24/outline';
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import Container from '@/components/ui/Container';
import Section from '@/components/ui/Section';

type ReportingSeriesResponse = {
  series: Array<{ timestamp: string; revenue: number; impressions: number; ecpm: number }>;
  period?: { startDate: string; endDate: string };
  granularity?: string;
};

type ReportingOverviewResponse = {
  totalRevenue: number;
  totalImpressions: number;
  totalClicks: number;
  ecpm: number;
  ctr: number;
  fillRate: number;
  period?: { startDate: string; endDate: string };
};

type RangeKey = 'today' | 'week' | 'month' | 'year';

type TimePoint = { ts: string; revenue: number; impressions: number; ecpm: number };

type RevenueSeries = {
  points: TimePoint[];
};

type TopApp = { name: string; revenue: number; impressions: number; percent: number };
type TopNetwork = { name: string; revenue: number; ecpm: number; percent: number };

type TopAppRow = { appId: string; appName?: string | null; revenue: number; impressions: number; ecpm: number };
type AdapterPerformanceRow = { adapterId: string; adapterName?: string | null; revenue: number; impressions: number; clicks: number; ecpm: number; ctr: number; avgLatency: number };

export default function RevenuePage() {
  const [timeRange, setTimeRange] = useState<RangeKey>('week');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [series, setSeries] = useState<RevenueSeries>({ points: [] });
  const [summary, setSummary] = useState<ReportingOverviewResponse | null>(null);
  const [topApps, setTopApps] = useState<TopApp[]>([]);
  const [topNetworks, setTopNetworks] = useState<TopNetwork[]>([]);

  // Localization helpers
  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const numFmt = useMemo(() => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }), []);
  const curFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'currency', currency }), [currency]);

  // Range mapping for API
  function toApiRange(r: RangeKey): { startDate: string; endDate: string } {
    const end = new Date();
    const start = new Date(end);
    switch (r) {
      case 'today':
        start.setDate(end.getDate() - 1);
        break;
      case 'week':
        start.setDate(end.getDate() - 7);
        break;
      case 'month':
        start.setMonth(end.getMonth() - 1);
        break;
      case 'year':
      default:
        start.setFullYear(end.getFullYear() - 1);
        break;
    }
    return {
      startDate: start.toISOString().slice(0, 10),
      endDate: end.toISOString().slice(0, 10),
    };
  }

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    setLoading(true);
    setError(null);
    (async () => {
      const { startDate, endDate } = toApiRange(timeRange);
      const query = `startDate=${startDate}&endDate=${endDate}`;
      // Fetch in parallel; tolerate missing routes by handling { success:false }
      const [seriesRes, summaryRes, appsRes, adaptersRes] = await Promise.all([
        api.get<ReportingSeriesResponse>(`/reporting/timeseries?${query}`, { signal: controller?.signal }),
        api.get<ReportingOverviewResponse>(`/reporting/overview?${query}`, { signal: controller?.signal }),
        api.get<{ apps: TopAppRow[] }>(`/reporting/top-apps?${query}`, { signal: controller?.signal }),
        api.get<{ adapters: AdapterPerformanceRow[] }>(`/reporting/adapters?${query}`, { signal: controller?.signal }),
      ]);

      if (!alive) return;
      if (!seriesRes.success || !seriesRes.data) {
        setError(seriesRes.error || 'Failed to load revenue data');
        setLoading(false);
        return;
      }
      setSeries({
        points: (seriesRes.data.series || []).map((point: any) => ({
          ts: point.timestamp,
          revenue: point.revenue,
          impressions: point.impressions,
          ecpm: point.ecpm,
        })),
      });
      setSummary(summaryRes.success && summaryRes.data ? summaryRes.data : null);

      const appRows = appsRes.success && appsRes.data?.apps ? appsRes.data.apps : [];
      const totalAppRevenue = appRows.reduce((sum, row) => sum + Math.max(0, row.revenue || 0), 0);
      setTopApps(
        appRows.map((row) => {
          const revenue = Math.max(0, row.revenue || 0);
          return {
            name: row.appName || row.appId || 'App',
            revenue,
            impressions: Math.max(0, row.impressions || 0),
            percent: totalAppRevenue > 0 ? (revenue / totalAppRevenue) * 100 : 0,
          };
        })
      );

      const adapterRows = adaptersRes.success && adaptersRes.data?.adapters ? adaptersRes.data.adapters : [];
      const totalAdapterRevenue = adapterRows.reduce((sum, row) => sum + Math.max(0, row.revenue || 0), 0);
      setTopNetworks(
        adapterRows.map((row) => {
          const revenue = Math.max(0, row.revenue || 0);
          return {
            name: row.adapterName || row.adapterId || 'Network',
            revenue,
            ecpm: Math.max(0, row.ecpm || 0),
            percent: totalAdapterRevenue > 0 ? (revenue / totalAdapterRevenue) * 100 : 0,
          };
        })
      );
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
  }, [timeRange]);

  // Derive current headline stats from the time series
  const totals = useMemo(() => {
    const revenue = series.points.reduce((acc, p) => acc + Math.max(0, p.revenue || 0), 0);
    const impressions = series.points.reduce((acc, p) => acc + Math.max(0, p.impressions || 0), 0);
    const ecpm = impressions > 0 ? (revenue / impressions) * 1000 : 0;
    return { revenue, impressions, ecpm };
  }, [series]);

  // Simple change vs previous segment (best‑effort if series contains at least 2 buckets)
  const changePct = useMemo(() => {
    if (series.points.length < 2) return 0;
    const mid = Math.floor(series.points.length / 2);
    const sum = (pts: TimePoint[]) => pts.reduce((a, p) => a + Math.max(0, p.revenue || 0), 0);
    const prev = sum(series.points.slice(0, mid));
    const curr = sum(series.points.slice(mid));
    if (prev <= 0) return curr > 0 ? 100 : 0;
    return ((curr - prev) / prev) * 100;
  }, [series]);

  return (
    <Section>
      <Container className="space-y-6">
      {/* Page Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">
            Revenue Dashboard
          </h1>
          <p className="text-sm text-gray-600 mt-1">
            Track your earnings and optimize your monetization strategy
          </p>
        </div>
        <div className="flex gap-2">
          {(['today', 'week', 'month', 'year'] as const).map((range) => (
            <button
              key={range}
              onClick={() => setTimeRange(range)}
              className={`px-4 py-2 text-sm font-semibold rounded border transition-colors ${
                timeRange === range
                  ? 'bg-brand-50 text-brand-700 border-brand-500'
                  : 'bg-white text-gray-700 border-gray-300 hover:border-brand-500'
              }`}
              aria-pressed={timeRange === range}
            >
              {range}
            </button>
          ))}
        </div>
      </div>

      {/* Error state */}
      {error && (
        <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
          {error}
        </div>
      )}

      {/* Stats Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
        {loading ? (
          <>
            <SkeletonCard />
            <SkeletonCard />
            <SkeletonCard />
            <SkeletonCard />
          </>
        ) : (
          <>
            <StatCard
              title="Total Revenue"
              value={curFmt.format(Math.max(0, summary?.totalRevenue ?? totals.revenue))}
              change={changePct}
              icon={CurrencyDollarIcon}
              color="yellow"
            />
            <StatCard
              title="Impressions"
              value={numFmt.format(Math.max(0, summary?.totalImpressions ?? totals.impressions))}
              change={undefined}
              icon={ChartBarIcon}
              color="blue"
            />
            <StatCard
              title="eCPM"
              value={curFmt.format(Math.max(0, summary?.ecpm ?? totals.ecpm))}
              change={undefined}
              icon={ArrowTrendingUpIcon}
              color="yellow"
            />
            <StatCard
              title="Next Payout"
              value={curFmt.format(0)}
              subtitle="Configure your payout details in Settings"
              icon={BanknotesIcon}
              color="blue"
            />
          </>
        )}
      </div>

      {/* Revenue Chart */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-semibold text-lg mb-6">
          Revenue Trend
        </h2>
        {loading ? (
          <div className="h-64 animate-pulse bg-gray-100 rounded" aria-hidden="true" />
        ) : series.points.length === 0 ? (
          <p className="text-sm text-gray-600">No data for this period.</p>
        ) : (
          <div className="h-64 flex items-end justify-between gap-2" aria-live="polite">
            {series.points.map((p, i) => {
              const value = Math.max(0, p.revenue || 0);
              // Normalize heights within this series (avoid division by zero)
              const maxVal = Math.max(
                1,
                ...series.points.map((pp) => Math.max(0, pp.revenue || 0))
              );
              const height = Math.round((value / maxVal) * 100);
              const label = new Date(p.ts).toLocaleDateString();
              return (
                <div key={i} className="flex-1 flex flex-col items-center gap-2" aria-label={`${label}: ${curFmt.format(value)}`}>
                  <div
                    className="w-full bg-brand-500 rounded-t transition-all hover:opacity-90"
                    style={{ height: `${height}%` }}
                    title={`${label} — ${curFmt.format(value)}`}
                  />
                  <span className="text-xs text-gray-600">{label}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Top Performing Apps & Networks */}
      <div className="grid md:grid-cols-2 gap-6">
        {/* Top Apps */}
        <div className="card p-6">
          <h2 className="text-gray-900 font-bold uppercase text-lg mb-4 border-b-2 border-gray-200 pb-2">
            Top Performing Apps
          </h2>
          {loading ? (
            <div className="space-y-3" aria-hidden="true">
              {[...Array(4)].map((_, i) => (
                <div key={i} className="h-12 bg-gray-100 animate-pulse rounded" />
              ))}
            </div>
          ) : topApps.length === 0 ? (
            <p className="text-sm text-gray-600">No app breakdown for this period.</p>
          ) : (
            <div className="space-y-3">
              {topApps.map((a, i) => (
                <AppRevenueItem
                  key={i}
                  name={a.name}
                  revenue={Math.max(0, a.revenue)}
                  percentage={Math.max(0, Math.min(100, a.percent))}
                  impressions={Math.max(0, a.impressions)}
                  curFmt={curFmt}
                />
              ))}
            </div>
          )}
        </div>

        {/* Top Networks */}
        <div className="card p-6">
          <h2 className="text-gray-900 font-bold uppercase text-lg mb-4 border-b-2 border-gray-200 pb-2">
            Top Ad Networks
          </h2>
          {loading ? (
            <div className="space-y-3" aria-hidden="true">
              {[...Array(4)].map((_, i) => (
                <div key={i} className="h-12 bg-gray-100 animate-pulse rounded" />
              ))}
            </div>
          ) : topNetworks.length === 0 ? (
            <p className="text-sm text-gray-600">No network breakdown for this period.</p>
          ) : (
            <div className="space-y-3">
              {topNetworks.map((n, i) => (
                <NetworkRevenueItem
                  key={i}
                  name={n.name}
                  revenue={Math.max(0, n.revenue)}
                  percentage={Math.max(0, Math.min(100, n.percent))}
                  ecpm={Math.max(0, n.ecpm)}
                  curFmt={curFmt}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Payout Information */}
      <div className="card-v2 p-6">
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-gray-900 font-semibold text-lg mb-2">
              Next Payout Schedule
            </h2>
            <p className="text-gray-700 text-body mb-4">
              Configure payout destination and threshold to enable automated transfers.
            </p>
            <div className="space-y-2 text-sm text-gray-700">
              <div className="flex items-center gap-2">
                <div className="w-2 h-2 bg-brand-500 rounded-full" />
                <span>Minimum threshold: configurable in Settings</span>
              </div>
              <div className="flex items-center gap-2">
                <div className="w-2 h-2 bg-brand-500 rounded-full" />
                <span>Supported methods depend on your region and currency</span>
              </div>
            </div>
          </div>
          <a
            href="/dashboard/settings"
            className="btn-primary px-6 py-2 text-sm"
          >
            Update Settings
          </a>
        </div>
      </div>
      </Container>
    </Section>
  );
}

interface StatCardProps {
  title: string;
  value: string;
  change?: number;
  subtitle?: string;
  icon: React.ComponentType<{ className?: string }>;
  color: 'yellow' | 'blue';
}

function StatCard({ title, value, change, subtitle, icon: Icon, color }: StatCardProps) {
  const bgColor = color === 'yellow' ? 'bg-brand-50' : 'bg-brand-600';
  const textColor = color === 'yellow' ? 'text-brand-700' : 'text-white';

  return (
    <div className="card p-6">
      <div className="flex items-start justify-between mb-4">
        <div>
          <p className="text-sm text-gray-600 font-medium uppercase">{title}</p>
          <h3 className="text-2xl font-bold text-gray-900 mt-1">{value}</h3>
          {subtitle && <p className="text-xs text-gray-500 mt-1">{subtitle}</p>}
        </div>
        <div className={`${bgColor} ${textColor} p-3 rounded`}>
          <Icon className="w-6 h-6" />
        </div>
      </div>
      {change !== undefined && (
        <div className="flex items-center gap-1 text-sm">
          <span className={change >= 0 ? 'text-success' : 'text-danger'}>
            {change >= 0 ? '↑' : '↓'} {Math.abs(change).toFixed(1)}%
          </span>
          <span className="text-gray-500">vs last period</span>
        </div>
      )}
    </div>
  );
}

interface AppRevenueItemProps {
  name: string;
  revenue: number;
  percentage: number;
  impressions: number;
  curFmt: Intl.NumberFormat;
}
function AppRevenueItem({ name, revenue, percentage, impressions, curFmt }: AppRevenueItemProps) {
  return (
    <div className="flex items-center justify-between py-2 border-b border-gray-100 last:border-0">
      <div className="flex-1">
        <p className="font-bold text-gray-900">{name}</p>
        <p className="text-xs text-gray-500">{impressions.toLocaleString()} impressions</p>
      </div>
      <div className="text-right">
        <p className="font-bold text-gray-900">{curFmt.format(revenue)}</p>
        <p className="text-xs text-gray-500">{percentage}%</p>
      </div>
    </div>
  );
}

interface NetworkRevenueItemProps {
  name: string;
  revenue: number;
  percentage: number;
  ecpm: number;
  curFmt: Intl.NumberFormat;
}

function NetworkRevenueItem({ name, revenue, percentage, ecpm, curFmt }: NetworkRevenueItemProps) {
  return (
    <div className="flex items-center justify-between py-2 border-b border-gray-100 last:border-0">
      <div className="flex-1">
        <p className="font-bold text-gray-900">{name}</p>
        <p className="text-xs text-gray-500">eCPM: {curFmt.format(ecpm)}</p>
      </div>
      <div className="text-right">
        <p className="font-bold text-gray-900">{curFmt.format(revenue)}</p>
        <p className="text-xs text-gray-500">{percentage}%</p>
      </div>
    </div>
  );
}

function SkeletonCard() {
  return (
    <div className="card p-6 animate-pulse" aria-hidden="true">
      <div className="h-4 w-24 bg-gray-200 rounded mb-3" />
      <div className="h-7 w-28 bg-gray-200 rounded mb-2" />
      <div className="h-3 w-20 bg-gray-200 rounded" />
    </div>
  );
}
