"use client";
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';

type PeriodKey = 'today' | 'yesterday' | 'thisWeek' | 'thisMonth';

type ReportingOverview = {
  totalRevenue: number;
  totalImpressions: number;
  totalClicks: number;
  ecpm: number;
  ctr: number;
  fillRate: number;
};

type OverviewEntry = {
  revenue: number;
  impressions: number;
  ecpm: number;
};

type Overview = Record<PeriodKey, OverviewEntry>;

const PERIOD_CONFIG: Array<{ key: PeriodKey; title: string; range: () => { start: Date; end: Date } }> = [
  {
    key: 'today',
    title: 'Today',
    range: () => {
      const end = new Date();
      const start = new Date(end);
      start.setHours(0, 0, 0, 0);
      return { start, end };
    },
  },
  {
    key: 'yesterday',
    title: 'Yesterday',
    range: () => {
      const today = new Date();
      const start = new Date(today);
      start.setDate(today.getDate() - 1);
      start.setHours(0, 0, 0, 0);
      const end = new Date(start);
      end.setDate(start.getDate() + 1);
      return { start, end };
    },
  },
  {
    key: 'thisWeek',
    title: 'This Week',
    range: () => {
      const end = new Date();
      const start = new Date(end);
      start.setDate(end.getDate() - 7);
      start.setHours(0, 0, 0, 0);
      return { start, end };
    },
  },
  {
    key: 'thisMonth',
    title: 'This Month',
    range: () => {
      const end = new Date();
      const start = new Date(end.getFullYear(), end.getMonth(), 1);
      start.setHours(0, 0, 0, 0);
      return { start, end };
    },
  },
];

function SkeletonCard() {
  return (
    <div className="border border-gray-200 rounded-lg p-4 animate-pulse" aria-hidden="true">
      <div className="h-4 w-20 bg-gray-200 rounded mb-3" />
      <div className="h-7 w-24 bg-gray-200 rounded mb-2" />
      <div className="flex justify-between">
        <div className="h-4 w-16 bg-gray-200 rounded" />
        <div className="h-4 w-16 bg-gray-200 rounded" />
      </div>
    </div>
  );
}

export default function RevenueOverview() {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<Overview | null>(null);

  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const numberFmt = useMemo(() => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }), []);
  const currencyFmt = useMemo(
    () => new Intl.NumberFormat(undefined, { style: 'currency', currency }),
    [currency]
  );

  useEffect(() => {
    let mounted = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    (async () => {
      const queries = await Promise.all(
        PERIOD_CONFIG.map(async ({ key, range }) => {
          const { start, end } = range();
          const params = new URLSearchParams({
            startDate: start.toISOString(),
            endDate: end.toISOString(),
          });
          const res = await api.get<ReportingOverview>(`/reporting/overview?${params.toString()}`, {
            signal: controller?.signal,
          });
          if (!res.success || !res.data) {
            throw new Error(res.error || `Failed to load ${key} stats`);
          }
          return { key, payload: res.data };
        })
      );
      if (!mounted) return;
      const next: Overview = PERIOD_CONFIG.reduce((acc, cfg) => {
        acc[cfg.key] = { revenue: 0, impressions: 0, ecpm: 0 };
        return acc;
      }, {} as Overview);
      queries.forEach(({ key, payload }) => {
        next[key] = {
          revenue: Math.max(0, payload.totalRevenue ?? 0),
          impressions: Math.max(0, payload.totalImpressions ?? 0),
          ecpm: Math.max(0, payload.ecpm ?? 0),
        };
      });
      setData(next);
      setLoading(false);
    })().catch((e) => {
      if (!mounted) return;
      setError(e?.message || 'Network error');
      setLoading(false);
    });
    return () => {
      mounted = false;
      controller?.abort();
    };
  }, []);

  return (
    <div className="bg-white shadow rounded-lg p-6">
      <h2 className="text-lg font-semibold text-gray-900 mb-4">Revenue Overview</h2>

      {loading ? (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6" aria-busy="true">
          {PERIOD_CONFIG.map((c) => (
            <SkeletonCard key={c.key} />
          ))}
        </div>
      ) : error ? (
        <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
          {error}
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
          {PERIOD_CONFIG.map(({ key, title }) => {
            const period = data![key];
            const revenue = Math.max(0, period?.revenue ?? 0);
            const impressions = Math.max(0, period?.impressions ?? 0);
            const ecpm = Math.max(0, period?.ecpm ?? 0);
            return (
              <TimeCard
                key={key}
                title={title}
                revenue={currencyFmt.format(revenue)}
                impressions={numberFmt.format(impressions)}
                ecpm={currencyFmt.format(ecpm)}
              />
            );
          })}
        </div>
      )}

      <div className="mt-6 text-center text-sm text-gray-500">
        <p>📊 Start generating revenue by adding your first app and placement</p>
      </div>
    </div>
  );
}

function TimeCard({
  title,
  revenue,
  impressions,
  ecpm,
}: {
  title: string;
  revenue: string;
  impressions: string;
  ecpm: string;
}) {
  return (
    <div className="border border-gray-200 rounded-lg p-4">
      <h3 className="text-sm font-medium text-gray-500 mb-3">{title}</h3>
      <div className="space-y-2">
        <div>
          <p className="text-2xl font-bold text-gray-900">{revenue}</p>
          <p className="text-xs text-gray-500">Revenue</p>
        </div>
        <div className="flex justify-between text-sm">
          <div>
            <p className="font-medium text-gray-700">{impressions}</p>
            <p className="text-xs text-gray-500">Impressions</p>
          </div>
          <div>
            <p className="font-medium text-gray-700">{ecpm}</p>
            <p className="text-xs text-gray-500">eCPM</p>
          </div>
        </div>
      </div>
    </div>
  );
}
