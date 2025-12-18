'use client';

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Networks Page"
// Ad Networks management page — wired to live APIs with resilient UX and accessibility

import {
    CheckCircleIcon,
    ExclamationTriangleIcon,
    PlusCircleIcon,
    XCircleIcon,
} from '@heroicons/react/24/outline';
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

type NetworkStatus = 'active' | 'inactive' | 'error';

interface NetworkRow {
  id: string;
  name: string;
  status: NetworkStatus;
  revenue: number;
  impressions: number;
  ecpm: number;
  fillRatePct: number;
  ctr: number;
  uptime?: number;
  healthScore?: number;
  note?: string;
}

type AdapterPerformanceRow = {
  adapterId: string;
  adapterName: string;
  revenue: number;
  impressions: number;
  clicks: number;
  ecpm: number;
  ctr: number;
  avgLatency: number;
};

type AdapterHealthRow = {
  adapterId: string;
  adapterName: string;
  healthScore: number;
  status: 'healthy' | 'degraded' | 'critical' | 'offline';
  uptime: number;
  errorRate: number;
  fillRate: number;
  revenueShare: number;
  lastIssue?: string;
};

const statusMap: Record<AdapterHealthRow['status'], NetworkStatus> = {
  healthy: 'active',
  degraded: 'active',
  critical: 'error',
  offline: 'inactive',
};

function normalize(perf: AdapterPerformanceRow, health?: AdapterHealthRow): NetworkRow {
  const fillRaw = health?.fillRate ?? 0;
  const fillRatePct = Math.max(0, Math.min(100, fillRaw <= 1 ? fillRaw * 100 : fillRaw));
  const ctrPct = Math.max(0, Math.min(100, perf.ctr ?? 0));
  const status = health ? statusMap[health.status] : 'inactive';
  return {
    id: perf.adapterId,
    name: perf.adapterName || 'Unnamed Network',
    status,
    revenue: Math.max(0, perf.revenue || 0),
    impressions: Math.max(0, perf.impressions || 0),
    ecpm: Math.max(0, perf.ecpm || 0),
    fillRatePct,
    ctr: ctrPct,
    uptime: health?.uptime,
    healthScore: health?.healthScore,
    note: health?.lastIssue,
  };
}

export default function NetworksPage() {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [rows, setRows] = useState<ReturnType<typeof normalize>[]>([]);

  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const curFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'currency', currency }), [currency]);
  const numFmt = useMemo(() => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }), []);

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    setLoading(true);
    setError(null);
    (async () => {
      const end = new Date();
      const start = new Date(end);
      start.setDate(end.getDate() - 7);
      start.setHours(0, 0, 0, 0);
      const params = new URLSearchParams({
        startDate: start.toISOString(),
        endDate: end.toISOString(),
      });
      const [performanceRes, healthRes] = await Promise.all([
        api.get<{ adapters: AdapterPerformanceRow[] }>(`/reporting/adapters?${params.toString()}`, { signal: controller?.signal }),
        api.get<AdapterHealthRow[]>('/reporting/adapters/health', { signal: controller?.signal }),
      ]);
      if (!alive) return;
      if (!performanceRes.success || !performanceRes.data) {
        setError(performanceRes.error || 'Failed to load adapter performance');
        setLoading(false);
        return;
      }
      const adapters = performanceRes.data.adapters ?? [];
      const healthList: AdapterHealthRow[] = (healthRes.success && Array.isArray(healthRes.data) ? healthRes.data : []) as AdapterHealthRow[];
      const healthMap = new Map<string, AdapterHealthRow>();
      healthList.forEach((entry: AdapterHealthRow) => {
        healthMap.set(entry.adapterId, entry);
      });
      const mapped: NetworkRow[] = adapters.map((adapter: AdapterPerformanceRow) => normalize(adapter, healthMap.get(adapter.adapterId)));
      // Include adapters that have health signals but no revenue in window
      healthList.forEach((health) => {
        if (!mapped.some((row) => row.id === health.adapterId)) {
          mapped.push(
            normalize(
              {
                adapterId: health.adapterId,
                adapterName: health.adapterName,
                revenue: 0,
                impressions: 0,
                clicks: 0,
                ecpm: 0,
                ctr: 0,
                avgLatency: 0,
              },
              health
            )
          );
        }
      });
      setRows(mapped);
      setLoading(false);
    })().catch((e) => {
      if (!alive) return;
      setError(e?.message || 'Network error');
      setLoading(false);
    });
    return () => { alive = false; controller?.abort(); };
  }, []);

  const activeNetworks = rows.filter((n) => n.status === 'active').length;
  const totalRevenue = rows.reduce((sum, n) => sum + n.revenue, 0);
  const totalImpressions = rows.reduce((sum, n) => sum + n.impressions, 0);

  return (
    <Section>
      <Container className="space-y-6">
      {/* Page Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">
            Ad Networks
          </h1>
          <p className="text-sm text-gray-600 mt-1">
            Manage your ad network integrations and monitor performance
          </p>
        </div>
        <button className="btn-primary flex items-center gap-2">
          <PlusCircleIcon className="w-5 h-5" />
          Add Network
        </button>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
              Active Networks
            </p>
            {loading ? (
              <div className="h-9 w-24 bg-gray-100 rounded animate-pulse" aria-hidden="true" />
            ) : (
              <>
                <p className="text-3xl font-semibold text-gray-900">{activeNetworks}</p>
                <p className="text-sm text-gray-600 mt-1">of {rows.length} total</p>
              </>
            )}
          </div>
        </div>
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
              Total Revenue (Week)
            </p>
            {loading ? (
              <div className="h-9 w-40 bg-gray-100 rounded animate-pulse" aria-hidden="true" />
            ) : (
              <>
                <p className="text-3xl font-semibold text-gray-900">{curFmt.format(Math.max(0, totalRevenue))}</p>
                <p className="text-sm text-gray-600 mt-1">across all networks</p>
              </>
            )}
          </div>
        </div>
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
              Total Impressions
            </p>
            {loading ? (
              <div className="h-9 w-40 bg-gray-100 rounded animate-pulse" aria-hidden="true" />
            ) : (
              <>
                <p className="text-3xl font-semibold text-gray-900">{numFmt.format(Math.max(0, totalImpressions))}</p>
                <p className="text-sm text-gray-600 mt-1">this week</p>
              </>
            )}
          </div>
        </div>
      </div>

      {/* Error/empty */}
      {error && (
        <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">{error}</div>
      )}

      {/* Networks Grid */}
      {loading ? (
        <div className="grid md:grid-cols-2 gap-6" aria-busy="true">
          {[...Array(4)].map((_, i) => (<div key={i} className="h-40 bg-gray-100 animate-pulse rounded" />))}
        </div>
      ) : rows.length === 0 ? (
        <p className="text-sm text-gray-600">No networks connected yet.</p>
      ) : (
        <div className="grid md:grid-cols-2 gap-6">
          {rows.map((network) => (
            <NetworkCard key={network.id} network={network} curFmt={curFmt} numFmt={numFmt} />
          ))}
        </div>
      )}

      {/* Integration Instructions */}
      <div className="card-v2">
        <div className="card-v2-header">
          <h2 className="text-sm font-semibold text-gray-900">How to Add a Network</h2>
        </div>
        <div className="card-v2-body">
          <ol className="space-y-3 text-sm text-gray-700">
            <li className="flex gap-3">
              <span className="inline-flex h-5 w-5 items-center justify-center rounded-full border text-xs font-semibold text-brand-700" style={{ borderColor: 'rgb(var(--brand-500))' }}>1</span>
              <span>
                Click the &ldquo;Add Network&rdquo; button above and select your ad network from the list
              </span>
            </li>
            <li className="flex gap-3">
              <span className="inline-flex h-5 w-5 items-center justify-center rounded-full border text-xs font-semibold text-brand-700" style={{ borderColor: 'rgb(var(--brand-500))' }}>2</span>
              <span>
                Enter your API credentials (App ID, API Key, etc.) from the network&rsquo;s dashboard
              </span>
            </li>
            <li className="flex gap-3">
              <span className="inline-flex h-5 w-5 items-center justify-center rounded-full border text-xs font-semibold text-brand-700" style={{ borderColor: 'rgb(var(--brand-500))' }}>3</span>
              <span>
                Configure bidding parameters: min eCPM, timeout, priority level
              </span>
            </li>
            <li className="flex gap-3">
              <span className="inline-flex h-5 w-5 items-center justify-center rounded-full border text-xs font-semibold text-brand-700" style={{ borderColor: 'rgb(var(--brand-500))' }}>4</span>
              <span>
                Test the connection and wait for the first ad request (usually within 5 minutes)
              </span>
            </li>
            <li className="flex gap-3">
              <span className="inline-flex h-5 w-5 items-center justify-center rounded-full border text-xs font-semibold text-brand-700" style={{ borderColor: 'rgb(var(--brand-500))' }}>5</span>
              <span>
                Monitor performance in real-time and adjust settings as needed
              </span>
            </li>
          </ol>
        </div>
      </div>
      </Container>
    </Section>
  );
}

interface NetworkCardProps {
  network: ReturnType<typeof normalize>;
  curFmt: Intl.NumberFormat;
  numFmt: Intl.NumberFormat;
}

function NetworkCard({ network, curFmt, numFmt }: NetworkCardProps) {
  const statusConfig = {
    active: {
      icon: CheckCircleIcon,
      color: 'text-success',
      bg: 'bg-success/10',
      label: 'Active',
    },
    inactive: {
      icon: XCircleIcon,
      color: 'text-gray-600',
      bg: 'bg-gray-50',
      label: 'Inactive',
    },
    error: {
      icon: ExclamationTriangleIcon,
      color: 'text-danger',
      bg: 'bg-danger/10',
      label: 'Error',
    },
  };

  const config = statusConfig[network.status];
  const StatusIcon = config.icon;
  const note = network.note ? `Issue: ${network.note}` : 'No recent incidents';

  return (
    <div className="card-v2">
      {/* Header */}
      <div className="card-v2-header">
        <div className="flex items-start justify-between w-full">
          <div className="flex-1">
            <h3 className="text-base md:text-lg font-semibold text-gray-900">{network.name}</h3>
            <div className={`mt-1 inline-flex items-center gap-2 ${config.color}`}>
              <StatusIcon className="w-4 h-4" />
              <span className="text-xs font-semibold">{config.label}</span>
            </div>
          </div>
          {network.status === 'active' && (
            <div className="inline-flex items-center rounded-full bg-brand-50 px-3 py-1 text-xs font-semibold uppercase text-brand-700 border" style={{ borderColor: 'rgb(var(--brand-500))' }}>
              Live
            </div>
          )}
        </div>
      </div>

      <div className="card-v2-body">
        {/* Metrics */}
        {network.status === 'active' ? (
          <div className="grid grid-cols-2 gap-4 mb-4">
            <div>
              <p className="text-xs text-gray-600 uppercase">Revenue (Week)</p>
              <p className="text-lg font-semibold text-gray-900">{curFmt.format(Math.max(0, network.revenue))}</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">Impressions</p>
              <p className="text-lg font-semibold text-gray-900">{numFmt.format(Math.max(0, totalOrZero(network.impressions)))}{/* helper inline to satisfy TS */}</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">eCPM</p>
              <p className="text-lg font-semibold text-gray-900">{curFmt.format(Math.max(0, network.ecpm))}</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">Fill Rate</p>
              <p className="text-lg font-semibold text-gray-900">{Math.round(network.fillRatePct)}%</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">CTR</p>
              <p className="text-lg font-semibold text-gray-900">{Math.round(network.ctr * 10) / 10}%</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">Health Score</p>
              <p className="text-lg font-semibold text-gray-900">{network.healthScore !== undefined ? Math.round(network.healthScore) : '—'}</p>
            </div>
          </div>
        ) : (
          <div className={`${config.bg} p-4 rounded mb-4`}>
            <p className="text-sm text-gray-700">
              {network.status === 'inactive' && 'This network is not connected. Click "Configure" to set up.'}
              {network.status === 'error' && 'Connection error. Check your credentials and try reconnecting. Review logs for details.'}
            </p>
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between pt-4 border-t border-gray-200">
          <div className="text-xs text-gray-500 space-y-1">
            {network.uptime !== undefined && (
              <p>Uptime {network.uptime.toFixed(1)}%</p>
            )}
            <p>{note}</p>
          </div>
          <a href="/docs/Adapters" className="btn-secondary text-sm">
            Configure
          </a>
        </div>
      </div>
    </div>
  );
}

// small helper for readability
function totalOrZero(n: number | undefined) { return Math.max(0, n || 0); }
