'use client';

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Fraud Detection Page"
// ML-powered fraud detection dashboard with live data wiring and accessible severity indicators

import {
    CheckCircleIcon,
    ExclamationTriangleIcon,
    ShieldCheckIcon,
    XCircleIcon,
} from '@heroicons/react/24/outline';
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

type Severity = 'high' | 'medium' | 'low'
type FraudType = 'click_fraud' | 'install_fraud' | 'bot_traffic' | 'vpn_abuse'

interface FraudEvent {
  id: string;
  ts: string; // ISO
  type: FraudType;
  severity: Severity;
  ip?: string;
  countryCode?: string; // ISO-2
  blocked: boolean;
  details?: string;
}

interface FraudSummary {
  totalDetected: number;
  blockedRevenue: number;
  detectionRate: number; // percentage 0..100
  avgFraudScore: number;
  topTypes: { type: string; count: number; percentage: number }[];
  lastDetectionAt?: string;
}

const typeLabels: Record<string, string> = {
  click_fraud: 'Click Fraud',
  install_fraud: 'Install Fraud',
  bot_traffic: 'Bot Traffic',
  vpn_abuse: 'VPN Abuse',
};

// Map fraud type to a semantic color used by FraudTypeBar
function typeColor(t: string): 'danger' | 'warning' | 'info' | 'gray' {
  switch (t as FraudType) {
    case 'click_fraud':
      return 'danger';
    case 'install_fraud':
      return 'warning';
    case 'bot_traffic':
      return 'info';
    case 'vpn_abuse':
      return 'gray';
    default:
      return 'gray';
  }
}

export default function FraudPage() {
  const [timeRange, setTimeRange] = useState<'today' | 'week' | 'month'>('today');

  // Data state
  const [summary, setSummary] = useState<FraudSummary | null>(null);
  const [events, setEvents] = useState<FraudEvent[]>([]);
  const [page, setPage] = useState(1);
  const [pageSize] = useState(20);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const numberFmt = useMemo(() => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }), []);
  const currencyFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'currency', currency: process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD' }), []);
  const percentFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'percent', maximumFractionDigits: 1 }), []);

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    setLoading(true);
    setError(null);
    (async () => {
      const limit = page * pageSize;
      const [statsRes, alertsRes] = await Promise.all([
        api.get<FraudSummary>('/fraud/stats', { signal: controller?.signal }),
        api.get<Array<{ id: string; type: string; severity: string; details?: string; detectedAt: string }>>(`/fraud/alerts?limit=${limit}`, { signal: controller?.signal }),
      ]);
      if (!alive) return;
      if (!statsRes.success || !statsRes.data) {
        setError(statsRes.error || 'Failed to load fraud summary');
        setLoading(false);
        return;
      }
      const alerts = alertsRes.success && Array.isArray(alertsRes.data) ? alertsRes.data : [];
      const start = (page - 1) * pageSize;
      const paged = alerts.slice(start, start + pageSize).map((row: { id: string; type: string; severity: string; details?: string; detectedAt: string }) => ({
        id: row.id,
        ts: row.detectedAt,
        type: (row.type as FraudType) || 'bot_traffic',
        severity: (row.severity as Severity) || 'low',
        blocked: false,
        details: row.details,
      }));
      setSummary(statsRes.data);
      setEvents(paged);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [timeRange, page, pageSize]);

  const fraudEvents = events;

  return (
    <Section>
      <Container className="space-y-6">
      {/* Page Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">
            Fraud Detection
          </h1>
          <p className="text-sm text-gray-600 mt-1">
            ML-powered fraud protection with live alerts and actionable summaries
          </p>
        </div>
        <div className="flex gap-2">
          {(['today', 'week', 'month'] as const).map((range) => (
            <button
              key={range}
              onClick={() => setTimeRange(range)}
              className={`px-4 py-2 text-sm font-semibold rounded border transition-colors ${
                timeRange === range
                  ? 'bg-brand-50 text-brand-700 border-brand-500'
                  : 'bg-white text-gray-700 border-gray-300 hover:border-brand-500'
              }`}
            >
              {range}
            </button>
          ))}
        </div>
      </div>

      {/* Summary cards */}
      <div className="card-v2 p-6">
        <div className="flex items-center gap-3 mb-6">
          <ShieldCheckIcon className="w-8 h-8 text-brand-600" />
          <div>
            <h2 className="text-gray-900 font-semibold text-lg">
              Fraud Summary
            </h2>
            <p className="text-gray-600 text-sm">Last {timeRange === 'today' ? '24 hours' : timeRange}</p>
          </div>
        </div>
        {loading ? (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-6" aria-busy="true">
            {[0,1,2,3].map((i) => (
              <div key={i} className="animate-pulse">
                <div className="h-8 w-24 bg-gray-200 rounded mb-2" />
                <div className="h-3 w-28 bg-gray-100 rounded" />
              </div>
            ))}
          </div>
        ) : error ? (
          <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
            {error}
          </div>
        ) : summary ? (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-6">
            <div>
              <p className="text-3xl font-semibold text-gray-900">{numberFmt.format(summary.totalDetected)}</p>
              <p className="text-sm text-gray-600 mt-1">Detections</p>
            </div>
            <div>
              <p className="text-3xl font-semibold text-gray-900">{currencyFmt.format(Math.max(0, summary.blockedRevenue))}</p>
              <p className="text-sm text-gray-600 mt-1">Blocked Revenue</p>
            </div>
            <div>
              <p className="text-3xl font-semibold text-gray-900">{percentFmt.format(Math.max(0, summary.detectionRate / 100))}</p>
              <p className="text-sm text-gray-600 mt-1">Detection Rate</p>
            </div>
            <div>
              <p className="text-3xl font-semibold text-gray-900">{summary.lastDetectionAt ? new Date(summary.lastDetectionAt).toLocaleTimeString() : '—'}</p>
              <p className="text-sm text-gray-600 mt-1">Last Detection</p>
            </div>
          </div>
        ) : null}
      </div>

      {/* Stats Cards (derived from summary when present) */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-6">
        <StatCard
          title="Detections"
          value={summary ? numberFmt.format(summary.totalDetected) : '—'}
          subtitle="Fraud attempts"
          color="danger"
          icon={XCircleIcon}
        />
        <StatCard
          title="Blocked Revenue"
          value={summary ? currencyFmt.format(Math.max(0, summary.blockedRevenue)) : '—'}
          subtitle="Current window"
          color="warning"
          icon={ShieldCheckIcon}
        />
        <StatCard
          title="Detection Rate"
          value={summary ? percentFmt.format(Math.max(0, summary.detectionRate / 100)) : '—'}
          subtitle="Of total traffic"
          color="info"
          icon={ExclamationTriangleIcon}
        />
        <StatCard
          title="Avg Fraud Score"
          value={summary ? summary.avgFraudScore.toFixed(2) : '—'}
          subtitle="0 (low) to 1 (high)"
          color="success"
          icon={CheckCircleIcon}
        />
      </div>

      {/* Fraud Type Breakdown */}
      <div className="grid md:grid-cols-2 gap-6">
        {/* Fraud Types */}
        <div className="card p-6">
          <h2 className="text-gray-900 font-bold uppercase text-lg mb-4 border-b-2 border-gray-200 pb-2">
            Fraud Type Breakdown
          </h2>
          <div className="space-y-4">
            {loading && (
              <div className="animate-pulse h-28 bg-gray-100 rounded" aria-hidden="true" />
            )}
              {!loading && summary && summary.topTypes.length === 0 && (
              <p className="text-sm text-gray-600">No fraud detected in this period.</p>
            )}
              {!loading && summary && summary.topTypes.length > 0 && (
                summary.topTypes.map((row) => (
                <FraudTypeBar
                  key={row.type}
                  type={typeLabels[row.type]}
                  count={row.count}
                    percentage={row.percentage}
                  color={typeColor(row.type)}
                />
              ))
            )}
          </div>
        </div>

        {/* Top Blocked Countries */}
        <div className="card p-6">
          <h2 className="text-gray-900 font-bold uppercase text-lg mb-4 border-b-2 border-gray-200 pb-2">
            Top Blocked Countries
          </h2>
          <div className="space-y-3">
            {loading && <div className="animate-pulse h-24 bg-gray-100 rounded" aria-hidden="true" />}
              <p className="text-sm text-gray-600">Country-level blocking data is not yet available.</p>
          </div>
        </div>
      </div>

      {/* Recent Fraud Events Timeline */}
      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          Recent Fraud Events
        </h2>
        {loading ? (
          <div className="space-y-3" aria-busy="true">
            {[...Array(5)].map((_, i) => (
              <div key={i} className="h-16 bg-gray-100 animate-pulse rounded" />
            ))}
          </div>
        ) : error ? (
          <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
            {error}
          </div>
        ) : fraudEvents.length === 0 ? (
          <p className="text-sm text-gray-600">No alerts for this period.</p>
        ) : (
          <div className="space-y-4" aria-live="polite">
            {fraudEvents.map((event) => (
              <FraudEventCard key={event.id} event={event} />
            ))}
            <Pagination
              page={page}
              onPageChange={(p) => setPage(Math.max(1, p))}
              hasNext={fraudEvents.length === pageSize}
            />
          </div>
        )}
      </div>

      {/* Model Features placeholder removed until backed by live telemetry */}
      </Container>
    </Section>
  );
}

interface StatCardProps {
  title: string;
  value: string;
  subtitle: string;
  color: 'danger' | 'warning' | 'info' | 'success';
  icon: React.ComponentType<{ className?: string }>;
}

function StatCard({ title, value, subtitle, color, icon: Icon }: StatCardProps) {
  const colorMap = {
    danger: 'bg-danger text-white',
    warning: 'bg-warning text-white',
    info: 'bg-info text-white',
    success: 'bg-success text-white',
  } as const;

  return (
    <div className="card p-6">
      <div className="flex items-start justify-between mb-4">
        <div>
          <p className="text-sm text-gray-600 font-medium uppercase">{title}</p>
          <h3 className="text-2xl font-bold text-gray-900 mt-1">{value}</h3>
          <p className="text-xs text-gray-500 mt-1">{subtitle}</p>
        </div>
        <div className={`${colorMap[color]} p-3 rounded`}>
          <Icon className="w-6 h-6" />
        </div>
      </div>
    </div>
  );
}

interface FraudTypeBarProps {
  type: string;
  count: number;
  percentage: number;
  color: 'danger' | 'warning' | 'info' | 'gray';
}

function FraudTypeBar({ type, count, percentage, color }: FraudTypeBarProps) {
  const colorMap = {
    danger: 'bg-danger',
    warning: 'bg-warning',
    info: 'bg-info',
    gray: 'bg-gray-400',
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-2">
        <span className="font-bold text-gray-900">{type}</span>
        <span className="text-sm text-gray-600">{count} blocked</span>
      </div>
      <div className="w-full bg-gray-200 h-6 rounded overflow-hidden">
        <div
          className={`${colorMap[color]} h-full flex items-center justify-end pr-2`}
          style={{ width: `${percentage}%` }}
        >
          <span className="text-white text-sm font-bold">{percentage}%</span>
        </div>
      </div>
    </div>
  );
}

interface FraudEventCardProps {
  event: FraudEvent;
}

function FraudEventCard({ event }: FraudEventCardProps) {
  const severityConfig = {
    high: { color: 'border-danger bg-danger/10', badge: 'bg-danger', label: 'High severity' },
    medium: { color: 'border-warning bg-warning/10', badge: 'bg-warning', label: 'Medium severity' },
    low: { color: 'border-gray-400 bg-gray-50', badge: 'bg-gray-400', label: 'Low severity' },
  };

  const config = severityConfig[event.severity];

  return (
    <div className={`border-l-4 ${config.color} p-4 rounded`} aria-label={`${config.label} ${typeLabels[event.type]} alert`}>
      <div className="flex items-start justify-between mb-2">
        <div className="flex items-center gap-3">
          <div className={`${config.badge} text-white px-3 py-1 rounded text-xs font-bold uppercase`}>
            {typeLabels[event.type]}
          </div>
          <span className="text-sm text-gray-600">{formatTs(event.ts)}</span>
        </div>
        {event.blocked ? (
          <div className="flex items-center gap-1 text-success text-sm font-bold">
            <CheckCircleIcon className="w-4 h-4" />
            Blocked
          </div>
        ) : (
          <div className="flex items-center gap-1 text-warning text-sm font-bold">
            <ExclamationTriangleIcon className="w-4 h-4" />
            Review
          </div>
        )}
      </div>
      {event.details && <p className="text-gray-700 text-sm mb-2">{event.details}</p>}
      <div className="flex items-center gap-4 text-xs text-gray-600">
        {event.ip && <span>IP: {event.ip}</span>}
        {event.countryCode && <span>Country: {event.countryCode}</span>}
      </div>
    </div>
  );
}

function Pagination({ page, hasNext, onPageChange }: { page: number; hasNext: boolean; onPageChange: (p: number) => void }) {
  return (
    <div className="flex items-center justify-end gap-2 pt-2">
      <button
        className="px-3 py-1 text-sm border rounded disabled:opacity-50"
        onClick={() => onPageChange(page - 1)}
        disabled={page <= 1}
        aria-label="Previous page"
      >
        Prev
      </button>
      <span className="text-sm" aria-live="polite">Page {page}</span>
      <button
        className="px-3 py-1 text-sm border rounded disabled:opacity-50"
        onClick={() => onPageChange(page + 1)}
        disabled={!hasNext}
        aria-label="Next page"
      >
        Next
      </button>
    </div>
  );
}

function formatTs(iso: string) {
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}
