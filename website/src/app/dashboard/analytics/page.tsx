"use client";

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Analytics Page"
// Analytics dashboard with resilient loading/error states and unified layout

import { ArrowPathIcon, ChartPieIcon, CurrencyDollarIcon, DevicePhoneMobileIcon, GlobeAltIcon, UsersIcon } from '@heroicons/react/24/outline';
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

type Range = 'today' | 'week' | 'month';

type AnalyticsOverview = {
  revenue: { today: number; yesterday: number; thisMonth: number; lastMonth: number; lifetime: number };
  impressions: number;
  clicks: number;
  ecpm: number;
  ctr: number;
};

type TimeSeriesPoint = { date: string; revenue: number; impressions: number; clicks: number };
type PerformanceRow = { adapter: string; requests: number; impressions: number; clicks: number; revenue: number; ecpm?: number; fillRate?: number; ctr?: number };

export default function AnalyticsPage() {
  const [timeRange, setTimeRange] = useState<Range>('week');
  const [overview, setOverview] = useState<AnalyticsOverview | null>(null);
  const [series, setSeries] = useState<TimeSeriesPoint[]>([]);
  const [performance, setPerformance] = useState<PerformanceRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const currencyFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'currency', currency }), [currency]);
  const numberFmt = useMemo(() => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }), []);

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    setLoading(true);
    setError(null);
    (async () => {
      const query = `?range=${timeRange}`;
      const [overviewRes, seriesRes, perfRes] = await Promise.all([
        api.get<AnalyticsOverview>(`/analytics/overview${query}`, { signal: controller?.signal }),
        api.get<{ points?: TimeSeriesPoint[] } | TimeSeriesPoint[]>(`/analytics/timeseries${query}`, { signal: controller?.signal }),
        api.get<{ items?: PerformanceRow[] } | PerformanceRow[]>(`/analytics/performance${query}`, { signal: controller?.signal }),
      ]);
      if (!alive) return;

      if (!seriesRes.success || !seriesRes.data) {
        setError(seriesRes.error || 'Failed to load analytics data');
        setLoading(false);
        return;
      }

      const seriesPayload = Array.isArray(seriesRes.data) ? seriesRes.data : seriesRes.data?.points || [];
      setSeries(
        (seriesPayload || []).map((p: any) => ({
          date: p.date || p.timestamp || new Date().toISOString(),
          revenue: sanitize(p.revenue),
          impressions: sanitize(p.impressions),
          clicks: sanitize(p.clicks),
        }))
      );
      setOverview(overviewRes.success && overviewRes.data ? overviewRes.data : null);
      const perfPayload = Array.isArray(perfRes.data) ? perfRes.data : perfRes.data?.items || [];
      setPerformance(
        (perfPayload || []).map((row: any) => ({
          adapter: row.adapter || row.adapterName || row.name || 'Unknown',
          requests: sanitize(row.requests),
          impressions: sanitize(row.impressions),
          clicks: sanitize(row.clicks),
          revenue: sanitize(row.revenue),
          ecpm: sanitize(row.ecpm),
          fillRate: sanitize(row.fillRate),
          ctr: sanitize(row.ctr),
        }))
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

  const totals = useMemo(
    () =>
      series.reduce(
        (acc, p) => ({
          revenue: acc.revenue + Math.max(0, p.revenue || 0),
          impressions: acc.impressions + Math.max(0, p.impressions || 0),
          clicks: acc.clicks + Math.max(0, p.clicks || 0),
        }),
        { revenue: 0, impressions: 0, clicks: 0 }
      ),
    [series]
  );

  const metricCards = useMemo(() => {
    if (!overview) return [] as Array<{ title: string; value: string; icon: any; change?: number }>;
    return [
      { title: 'Revenue (Today)', value: currencyFmt.format(Math.max(0, overview.revenue.today)), icon: CurrencyDollarIcon },
      { title: 'Revenue (This Month)', value: currencyFmt.format(Math.max(0, overview.revenue.thisMonth)), icon: ArrowPathIcon },
      { title: 'Lifetime Revenue', value: currencyFmt.format(Math.max(0, overview.revenue.lifetime)), icon: GlobeAltIcon },
      { title: 'Impressions', value: numberFmt.format(Math.max(0, overview.impressions)), icon: DevicePhoneMobileIcon },
      { title: 'Clicks', value: numberFmt.format(Math.max(0, overview.clicks)), icon: UsersIcon },
      { title: 'Avg eCPM', value: currencyFmt.format(Math.max(0, overview.ecpm)), icon: ChartPieIcon },
    ];
  }, [overview, currencyFmt, numberFmt]);

  return (
    <Section>
      <Container className="space-y-6">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">Analytics Dashboard</h1>
            <p className="text-sm text-gray-600 mt-1">Comprehensive insights into your ad performance and user engagement</p>
          </div>
          <div className="flex gap-2">
            {(['today', 'week', 'month'] as const).map((range) => (
              <button
                key={range}
                onClick={() => setTimeRange(range)}
                className={`px-4 py-2 text-sm font-semibold rounded border transition-colors ${
                  timeRange === range ? 'bg-brand-50 text-brand-700 border-brand-500' : 'bg-white text-gray-700 border-gray-300 hover:border-brand-500'
                }`}
                aria-pressed={timeRange === range}
              >
                {range}
              </button>
            ))}
          </div>
        </div>

        {error && (
          <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
            {error}
          </div>
        )}

        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {loading
            ? [...Array(6)].map((_, i) => (
                <div key={i} className="card-v2 p-6 animate-pulse" aria-hidden="true">
                  <div className="h-4 w-24 bg-gray-200 rounded mb-3" />
                  <div className="h-7 w-28 bg-gray-200 rounded mb-2" />
                  <div className="h-3 w-20 bg-gray-200 rounded" />
                </div>
              ))
            : metricCards.map((card) => (
                <div key={card.title} className="card-v2 p-6">
                  <div className="flex items-start justify-between">
                    <div>
                      <p className="text-sm text-gray-600 font-medium uppercase">{card.title}</p>
                      <p className="text-2xl font-semibold text-gray-900 mt-1">{card.value}</p>
                    </div>
                    <div className="bg-brand-50 text-brand-700 p-3 rounded">
                      <card.icon className="w-6 h-6" />
                    </div>
                  </div>
                </div>
              ))}
        </div>

        <div className="card-v2 p-6">
          <div className="flex items-center justify-between mb-4">
            <div>
              <h2 className="text-gray-900 font-semibold text-lg">Revenue Performance</h2>
              <p className="text-sm text-gray-600">Last {timeRange === 'today' ? '24h' : timeRange}</p>
            </div>
            <div className="text-right">
              <p className="text-xs text-gray-500">Total Revenue</p>
              <p className="text-2xl font-semibold text-gray-900">{currencyFmt.format(Math.max(0, totals.revenue))}</p>
            </div>
          </div>
          {loading ? (
            <div className="h-48 animate-pulse bg-gray-100 rounded" aria-hidden="true" />
          ) : series.length === 0 ? (
            <p className="text-sm text-gray-600">No data available for this range.</p>
          ) : (
            <SimpleBarChart series={series} currencyFormatter={currencyFmt} />
          )}
        </div>

        <div className="card-v2 p-6">
          <h2 className="text-gray-900 font-semibold text-lg mb-4 border-b border-gray-200 pb-2">Adapter Performance</h2>
          {loading ? (
            <div className="space-y-3" aria-busy="true">
              {[...Array(4)].map((_, i) => (
                <div key={i} className="h-12 bg-gray-100 animate-pulse rounded" />
              ))}
            </div>
          ) : performance.length === 0 ? (
            <p className="text-sm text-gray-600">No adapter data for this range.</p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-sm">
                <thead>
                  <tr className="text-gray-500 uppercase text-xs border-b">
                    <th scope="col" className="py-3">Adapter</th>
                    <th scope="col" className="py-3">Requests</th>
                    <th scope="col" className="py-3">Impressions</th>
                    <th scope="col" className="py-3">Clicks</th>
                    <th scope="col" className="py-3">eCPM</th>
                    <th scope="col" className="py-3">Revenue</th>
                  </tr>
                </thead>
                <tbody>
                  {performance.map((row) => (
                    <tr key={row.adapter} className="border-b last:border-0">
                      <td className="py-3 font-semibold text-gray-900">{row.adapter}</td>
                      <td className="py-3">{row.requests.toLocaleString()}</td>
                      <td className="py-3">{row.impressions.toLocaleString()}</td>
                      <td className="py-3">{row.clicks.toLocaleString()}</td>
                      <td className="py-3">{row.ecpm ? currencyFmt.format(Math.max(0, row.ecpm)) : '—'}</td>
                      <td className="py-3">{currencyFmt.format(Math.max(0, row.revenue))}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </Container>
    </Section>
  );
}

function sanitize(value?: number) {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

function SimpleBarChart({ series, currencyFormatter }: { series: TimeSeriesPoint[]; currencyFormatter: Intl.NumberFormat }) {
  const maxRevenue = Math.max(...series.map((p) => Math.max(0, p.revenue)), 1);
  return (
    <div className="h-48 flex items-end justify-between gap-2" aria-live="polite">
      {series.map((p, i) => {
        const value = Math.max(0, p.revenue || 0);
        const height = Math.round((value / maxRevenue) * 100);
        const label = new Date(p.date).toLocaleDateString();
        return (
          <div key={i} className="flex-1 flex flex-col items-center gap-2" aria-label={`${label}: ${currencyFormatter.format(value)}`}>
            <div className="w-full bg-brand-500 rounded-t transition-all hover:opacity-80" style={{ height: `${height}%` }} title={`${label} — ${currencyFormatter.format(value)}`} />
            <span className="text-xs text-gray-500 font-bold">{label}</span>
          </div>
        );
      })}
    </div>
  );
}
