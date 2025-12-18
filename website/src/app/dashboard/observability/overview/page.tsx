"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { auctionApi } from "@/lib/auctionApi";
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

type LatBins = [number, number, number, number, number, number, number, number];

interface TimeSeriesBucket {
  start_unix: number;
  duration_s: number;
  requests: number;
  success: number;
  no_fill: number;
  timeout: number;
  errors?: Record<string, number>;
  lat_bins: LatBins;
}

interface AdapterSeriesSnapshot {
  adapter: string;
  buckets: TimeSeriesBucket[];
}

interface SLOStatus {
  adapter: string;
  window: string;
  latency_p99_ms: number;
  error_rate: number;
  fill_rate: number;
  level: "OK" | "WARN" | "CRIT";
}

export default function ObservabilityOverviewPage() {
  const [series, setSeries] = useState<AdapterSeriesSnapshot[] | null>(null);
  const [slo, setSlo] = useState<{ window_1h: SLOStatus[]; window_24h: SLOStatus[] } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [days, setDays] = useState(7);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);

  // Keep track of polling/backoff
  const pollingRef = useRef<boolean>(true);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const backoffRef = useRef<number>(5000); // start at 5s

  const loadOnce = async (signal?: AbortSignal) => {
    const [ts, s] = await Promise.all([
      auctionApi.getAdapterMetricsTimeSeries(days, { signal }),
      auctionApi.getAdapterSLO({ signal }),
    ]);
    if (!ts.success) {
      setError(ts.error || "Failed to load time series");
      setSeries(null);
      return false;
    }
    if (!s.success) {
      setError(s.error || "Failed to load SLO status");
      setSlo(null);
      return false;
    }
    setError(null);
    setSeries((ts.data as AdapterSeriesSnapshot[]) || []);
    setSlo((s.data as any) || null);
    setLastUpdated(Date.now());
    return true;
  };

  const scheduleNext = (success: boolean) => {
    // Reset or increase backoff
    backoffRef.current = success ? 5000 : Math.min(60000, Math.max(5000, backoffRef.current * 2));
    if (!pollingRef.current) return;
    timeoutRef.current = setTimeout(async () => {
      if (!pollingRef.current) return;
      const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
      const ok = await loadOnce(controller?.signal).catch(() => false);
      scheduleNext(!!ok);
    }, backoffRef.current);
  };

  const reload = async () => {
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    const ok = await loadOnce(controller?.signal).catch(() => false);
    scheduleNext(!!ok);
  };

  useEffect(() => {
    let alive = true;
    pollingRef.current = true;
    setLoading(true);
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    loadOnce(controller?.signal)
      .then((ok) => {
        if (!alive) return;
        setLoading(false);
        scheduleNext(!!ok);
      })
      .catch(() => {
        if (!alive) return;
        setLoading(false);
        scheduleNext(false);
      });

    // Pause/resume on tab visibility change
    const onVis = () => {
      const hidden = typeof document !== 'undefined' && document.hidden;
      pollingRef.current = !hidden;
      if (!hidden) {
        // Immediately refresh when returning to visible
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
        const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
        loadOnce(ctrl?.signal).then((ok) => scheduleNext(!!ok)).catch(() => scheduleNext(false));
      } else if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
    };
    document.addEventListener('visibilitychange', onVis);

    return () => {
      alive = false;
      pollingRef.current = false;
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      document.removeEventListener('visibilitychange', onVis);
      controller?.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [days]);

  const hasCritOrWarn = useMemo(() => {
    const statuses = slo?.window_1h || [];
    return statuses.some((s) => s.level === "CRIT" || s.level === "WARN");
  }, [slo]);

  return (
    <Section>
      <Container className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-h2-sm font-bold uppercase text-gray-900 tracking-tight">Observability Overview</h1>
        <div className="flex items-center gap-2">
          <select
            value={days}
            onChange={(e) => setDays(Number(e.target.value))}
            className="input-v2 w-auto text-sm px-3 py-2"
            aria-label="Time range (days)"
          >
            {[1, 3, 7, 14].map((d) => (
              <option key={d} value={d}>{d}d</option>
            ))}
          </select>
          <button onClick={reload} className="btn-secondary text-sm px-4">Refresh</button>
        </div>
      </div>

      {hasCritOrWarn && (
        <div className="rounded border border-warning bg-warning/10 p-3 text-warning">
          One or more adapters are breaching SLOs. See badges below; investigate via the Mediation Debugger.
        </div>
      )}

      {error && <div className="rounded border border-danger/20 bg-danger/10 p-3 text-danger" role="alert" aria-live="polite">{error}</div>}
      {loading && <div aria-busy="true">Loading…</div>}

      {!loading && series && series.length > 0 && (
        <div className="grid grid-cols-1 gap-6 md:grid-cols-2 xl:grid-cols-3">
          {series.map((s) => (
            <AdapterCard key={s.adapter} snapshot={s} slo={pickSLO(s.adapter, slo)} />
          ))}
        </div>
      )}

      {!loading && series && series.length === 0 && (
        <div className="text-gray-600">No data yet. Trigger some adapter requests and try again.</div>
      )}

      {/* Aria-live summary for screen readers */}
      <div className="sr-only" aria-live="polite">
        {slo && (
          <>SLO status updated {lastUpdated ? new Date(lastUpdated).toLocaleTimeString() : ''}. {slo.window_1h?.length || 0} adapters monitored. {
            (slo.window_1h || []).filter(s => s.level === 'CRIT').length
          } critical and {(slo.window_1h || []).filter(s => s.level === 'WARN').length} warnings.</>
        )}
      </div>
      </Container>
    </Section>
  );
}

function pickSLO(adapter: string, slo: { window_1h: SLOStatus[]; window_24h: SLOStatus[] } | null) {
  if (!slo) return null;
  const s1 = slo.window_1h.find((s) => s.adapter === adapter);
  const s24 = slo.window_24h.find((s) => s.adapter === adapter);
  return { s1, s24 };
}

function AdapterCard({ snapshot, slo }: { snapshot: AdapterSeriesSnapshot; slo: any }) {
  const buckets = snapshot.buckets;
  const p95 = buckets.map((b) => estimateP95(b.lat_bins));
  const reqs = buckets.map((b) => b.requests);
  const last = buckets[buckets.length - 1];
  const totals = buckets.reduce(
    (acc, b) => {
      acc.requests += b.requests;
      acc.success += b.success;
      acc.no_fill += b.no_fill;
      acc.timeout += b.timeout;
      acc.errors += Object.values(b.errors || {}).reduce((a, v) => a + v, 0);
      return acc;
    },
    { requests: 0, success: 0, errors: 0, no_fill: 0, timeout: 0 }
  );
  const errRate = totals.requests ? totals.errors / totals.requests : 0;
  const fillRate = totals.requests ? totals.success / totals.requests : 0;

  return (
    <div className="rounded border p-4">
      <div className="flex items-center justify-between">
        <div className="text-lg font-semibold">{snapshot.adapter}</div>
        <div>{slo?.s1 && <SLOBadge level={slo.s1.level} />}</div>
      </div>
      <div className="mt-2 grid grid-cols-2 gap-4 text-sm">
        <div>
          <div className="text-gray-500">p95 latency (ms)</div>
          <Sparkline data={p95} color="rgb(var(--brand-600))" />
        </div>
        <div>
          <div className="text-gray-500">Requests</div>
          <Sparkline data={reqs.map((v) => v || 0)} color="rgb(var(--success))" />
        </div>
        <div>
          <div className="text-gray-500">Error rate</div>
          <div className="font-mono">{(errRate * 100).toFixed(2)}%</div>
        </div>
        <div>
          <div className="text-gray-500">Fill rate</div>
          <div className="font-mono">{(fillRate * 100).toFixed(2)}%</div>
        </div>
      </div>
      <div className="mt-2 text-xs text-gray-500">Last bucket p95 ≈ {estimateP95(last?.lat_bins || [0,0,0,0,0,0,0,0]).toFixed(0)} ms</div>
    </div>
  );
}

function SLOBadge({ level }: { level: "OK" | "WARN" | "CRIT" }) {
  const map = {
    OK: "bg-success/10 text-success border-success/20",
    WARN: "bg-warning/10 text-warning border-warning/20",
    CRIT: "bg-danger/10 text-danger border-danger/20",
  } as const;
  return <span className={`rounded border px-2 py-0.5 text-xs font-semibold ${map[level]}`}>{level}</span>;
}

function Sparkline({ data, color = "rgb(var(--muted))", width = 240, height = 60 }: { data: number[]; color?: string; width?: number; height?: number }) {
  const path = useMemo(() => {
    if (!data || data.length === 0) return "";
    const max = Math.max(...data, 1);
    const min = Math.min(...data, 0);
    const range = Math.max(max - min, 1e-9);
    const step = width / Math.max(1, data.length - 1);
    const points = data.map((v, i) => {
      const x = i * step;
      const y = height - ((v - min) / range) * height;
      return `${x},${y}`;
    });
    return `M ${points[0]} L ${points.slice(1).join(" ")}`;
  }, [data, width, height]);

  return (
    <svg width={width} height={height} role="img" aria-label="sparkline chart" className="block">
      <path d={path} stroke={color} fill="none" strokeWidth={2} />
    </svg>
  );
}

function estimateP95(bins: LatBins): number {
  const bounds = [25, 50, 100, 200, 400, 800, 1600, 3200];
  const total = bins.reduce((a, v) => a + v, 0);
  if (total <= 0) return 0;
  const threshold = Math.floor(total * 0.95);
  let cum = 0;
  for (let i = 0; i < bins.length; i++) {
    cum += bins[i] || 0;
    if (cum >= threshold) return bounds[i];
  }
  return bounds[bounds.length - 1];
}
