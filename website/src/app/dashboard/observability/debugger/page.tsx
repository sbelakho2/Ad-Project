'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import { auctionApi } from '@/lib/auctionApi';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

interface DebugEvent {
  placement_id: string;
  request_id: string;
  adapter: string;
  outcome: 'success' | 'no_bid';
  reason?: string;
  timings_ms?: Record<string, number>;
  req_summary?: Record<string, any>;
  resp_summary?: Record<string, any>;
  created_at: string;
}

export default function MediationDebuggerPage() {
  const [placementId, setPlacementId] = useState('');
  const [limit, setLimit] = useState(50);
  const [events, setEvents] = useState<DebugEvent[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);

  // Polling/backoff
  const pollingRef = useRef<boolean>(true);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const backoffRef = useRef<number>(5000);

  const loadOnce = async (signal?: AbortSignal) => {
    const res = await auctionApi.getMediationDebugEvents(placementId, limit, { signal });
    if (!res.success) {
      setError(res.error || 'Failed to load debug events');
      setEvents(null);
      return false;
    }
    setError(null);
    setEvents((res.data as DebugEvent[]) || []);
    setLastUpdated(Date.now());
    return true;
  };

  const scheduleNext = (success: boolean) => {
    backoffRef.current = success ? 5000 : Math.min(60000, Math.max(5000, backoffRef.current * 2));
    if (!pollingRef.current) return;
    timeoutRef.current = setTimeout(async () => {
      if (!pollingRef.current) return;
      const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
      const ok = await loadOnce(ctrl?.signal).catch(() => false);
      scheduleNext(!!ok);
    }, backoffRef.current);
  };

  const reload = async () => {
    const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    const ok = await loadOnce(ctrl?.signal).catch(() => false);
    scheduleNext(!!ok);
  };

  useEffect(() => {
    let alive = true;
    pollingRef.current = true;
    setLoading(true);
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    loadOnce(controller?.signal).then((ok) => {
      if (!alive) return;
      setLoading(false);
      scheduleNext(!!ok);
    }).catch(() => {
      if (!alive) return;
      setLoading(false);
      scheduleNext(false);
    });

    const onVis = () => {
      const hidden = typeof document !== 'undefined' && document.hidden;
      pollingRef.current = !hidden;
      if (!hidden) {
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
  }, [placementId, limit]);

  const grouped = useMemo(() => {
    if (!events) return {} as Record<string, DebugEvent[]>;
    // Redact PII-like fields in summaries before grouping
    const redactKeys = ['ip', 'ip_address', 'device_id', 'gaid', 'idfa', 'email'];
    const redact = (obj: any): any => {
      if (!obj || typeof obj !== 'object') return obj;
      if (Array.isArray(obj)) return obj.map(redact);
      const out: any = {};
      for (const [k, v] of Object.entries(obj)) {
        out[k] = redactKeys.includes(k) ? '[redacted]' : redact(v as any);
      }
      return out;
    };
    const sanitized: DebugEvent[] = events.map((e) => ({
      ...e,
      req_summary: redact(e.req_summary || {}),
      resp_summary: redact(e.resp_summary || {}),
    }));
    return sanitized.reduce((acc, ev) => {
      const key = ev.adapter;
      (acc[key] = acc[key] || []).push(ev);
      return acc;
    }, {} as Record<string, DebugEvent[]>);
  }, [events]);

  return (
    <Section>
      <Container className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-h2-sm font-bold uppercase text-gray-900 tracking-tight">Mediation Debugger</h1>
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={placementId}
            onChange={(e) => setPlacementId(e.target.value)}
            placeholder="Placement ID (optional)"
            className="input-v2 w-64 text-sm"
          />
          <select
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value))}
            className="input-v2 w-auto text-sm px-3 py-2"
          >
            {[25, 50, 100, 200].map((n) => (
              <option key={n} value={n}>{n}</option>
            ))}
          </select>
          <button onClick={reload} className="btn-secondary text-sm px-4">Refresh</button>
        </div>
      </div>

      {error && <div className="rounded border border-danger/20 bg-danger/10 p-3 text-danger" role="alert" aria-live="polite">{error}</div>}
      {loading && <div aria-busy="true">Loading…</div>}

      {!loading && events && events.length === 0 && (
        <div className="text-gray-600">No debug events yet. Trigger an auction and try again.</div>
      )}

      {!loading && events && events.length > 0 && (
        <div className="space-y-6">
          {Object.entries(grouped).map(([adapter, list]) => (
            <div key={adapter} className="rounded border">
              <div className="border-b bg-gray-50 p-3 text-sm font-semibold">
                {adapter} — Last {list.length} events
              </div>
              <div className="divide-y">
                {list.map((_, i) => {
                  const idx = list.length - 1 - i; // render newest first without array reversal
                  const ev = list[idx];
                  return (
                  <div key={idx} className="grid grid-cols-12 gap-2 p-3 text-sm">
                    <div className="col-span-2">
                      <div className="text-gray-500">Time</div>
                      <div className="font-mono">{fmtTime(ev.created_at)}</div>
                    </div>
                    <div className="col-span-2">
                      <div className="text-gray-500">Outcome</div>
                      <OutcomeChip outcome={ev.outcome} reason={ev.reason} />
                    </div>
                    <div className="col-span-3">
                      <div className="text-gray-500">Req</div>
                      <CodeBlock obj={{ placement_id: ev.placement_id, request_id: ev.request_id, ...(ev.req_summary || {}) }} />
                    </div>
                    <div className="col-span-3">
                      <div className="text-gray-500">Resp</div>
                      <CodeBlock obj={ev.resp_summary || {}} />
                    </div>
                    <div className="col-span-2">
                      <div className="text-gray-500">Timings (ms)</div>
                      <CodeInline obj={ev.timings_ms || {}} />
                    </div>
                  </div>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Screen reader summary */}
      <div className="sr-only" aria-live="polite">
        {lastUpdated && `Debugger events updated ${new Date(lastUpdated).toLocaleTimeString()}.`}
      </div>
      </Container>
    </Section>
  );
}

function fmtTime(iso: string) {
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}

function OutcomeChip({ outcome, reason }: { outcome: 'success' | 'no_bid'; reason?: string }) {
  const color = outcome === 'success' ? 'bg-success/10 text-success border-success/20' : 'bg-warning/10 text-warning border-warning/20';
  return (
    <div className="flex items-center gap-2">
      <span className={`inline-block rounded border px-2 py-0.5 text-xs font-semibold ${color}`}>{outcome}</span>
      {reason && <span className="text-xs text-gray-600">{reason}</span>}
    </div>
  );
}

function CodeBlock({ obj }: { obj: any }) {
  return (
    <pre className="max-h-40 overflow-auto rounded bg-gray-50 p-2 font-mono text-xs text-gray-800">
      {JSON.stringify(obj, null, 2)}
    </pre>
  );
}

function CodeInline({ obj }: { obj: any }) {
  return (
    <code className="rounded bg-gray-50 px-2 py-1 font-mono text-xs text-gray-800">
      {JSON.stringify(obj)}
    </code>
  );
}
