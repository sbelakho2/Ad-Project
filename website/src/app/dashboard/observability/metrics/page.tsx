'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { auctionApi } from '@/lib/auctionApi';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

interface AdapterMetricsSnapshot {
  adapter: string;
  requests: number;
  success: number;
  no_fill: number;
  timeout: number;
  errors?: Record<string, number>;
  latency_p50_ms: number;
  latency_p95_ms: number;
  latency_p99_ms: number;
}

export default function AdapterMetricsPage() {
  const [data, setData] = useState<AdapterMetricsSnapshot[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);

  // Polling/backoff controls
  const pollingRef = useRef<boolean>(true);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const backoffRef = useRef<number>(5000); // start at 5s

  const loadOnce = useCallback(async (signal?: AbortSignal) => {
    const res = await auctionApi.getAdapterMetrics({ signal });
    if (!res.success) {
      setError(res.error || 'Failed to load metrics');
      setData(null);
      return false;
    }
    setError(null);
    setData(res.data as AdapterMetricsSnapshot[]);
    setLastUpdated(Date.now());
    return true;
  }, []);

  const scheduleNext = useCallback((success: boolean) => {
    backoffRef.current = success ? 5000 : Math.min(60000, Math.max(5000, backoffRef.current * 2));
    if (!pollingRef.current) return;
    timeoutRef.current = setTimeout(async () => {
      if (!pollingRef.current) return;
      const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
      const ok = await loadOnce(controller?.signal).catch(() => false);
      scheduleNext(!!ok);
    }, backoffRef.current);
  }, [loadOnce]);

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
  }, [loadOnce, scheduleNext]);

  return (
    <Section>
      <Container className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-h2-sm font-bold uppercase text-gray-900 tracking-tight">Adapter Metrics</h1>
        <button
          onClick={() => {
            if (timeoutRef.current) {
              clearTimeout(timeoutRef.current);
              timeoutRef.current = null;
            }
            const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
            setLoading(true);
            loadOnce(ctrl?.signal)
              .then((ok) => { setLoading(false); scheduleNext(!!ok); })
              .catch(() => { setLoading(false); scheduleNext(false); });
          }}
          className="btn-secondary text-sm px-4"
        >
          Refresh
        </button>
      </div>

      {error && (
        <div className="rounded border border-danger/20 bg-danger/10 p-3 text-danger" role="alert" aria-live="polite">{error}</div>
      )}

      {loading && <div aria-busy="true">Loading…</div>}

      {!loading && data && (
        <div className="overflow-x-auto rounded border">
          <table className="min-w-full divide-y divide-gray-200" aria-describedby="metrics-caption">
            <caption id="metrics-caption" className="sr-only">
              Adapter-level request, success, error, and latency metrics{lastUpdated ? `, updated ${new Date(lastUpdated).toLocaleTimeString()}` : ''}.
            </caption>
            <thead className="bg-gray-50">
              <tr>
                <Th>Adapter</Th>
                <Th>Requests</Th>
                <Th>Success</Th>
                <Th>No Fill</Th>
                <Th>Timeout</Th>
                <Th>Error Rate</Th>
                <Th>p50 (ms)</Th>
                <Th>p95 (ms)</Th>
                <Th>p99 (ms)</Th>
                <Th>Errors (by reason)</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-100 bg-white">
              {data.map((row) => {
                const errorsTotal = row.errors ? Object.values(row.errors).reduce((a, b) => a + b, 0) : 0;
                const errorRate = row.requests ? ((errorsTotal / row.requests) * 100).toFixed(2) + '%' : '0%';
                return (
                  <tr key={row.adapter}>
                    <Td>{row.adapter}</Td>
                    <Td>{row.requests}</Td>
                    <Td>{row.success}</Td>
                    <Td>{row.no_fill}</Td>
                    <Td>{row.timeout}</Td>
                    <Td>{errorRate}</Td>
                    <Td>{row.latency_p50_ms.toFixed(1)}</Td>
                    <Td>{row.latency_p95_ms.toFixed(1)}</Td>
                    <Td>{row.latency_p99_ms.toFixed(1)}</Td>
                    <Td>
                      {row.errors && Object.entries(row.errors).length > 0 ? (
                        <ul className="list-disc pl-5">
                          {Object.entries(row.errors).map(([reason, count]) => (
                            <li key={reason} className="text-sm text-gray-700">
                              <span className="font-medium">{reason}</span>: {count}
                            </li>
                          ))}
                        </ul>
                      ) : (
                        <span className="text-gray-400">—</span>
                      )}
                    </Td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {!loading && !error && (!data || data.length === 0) && (
        <div className="text-gray-600">No metrics available yet. Trigger some adapter requests and try again.</div>
      )}
      </Container>
    </Section>
  );
}

function Th({ children }: { children: React.ReactNode }) {
  return (
    <th scope="col" className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-700">
      {children}
    </th>
  );
}

function Td({ children }: { children: React.ReactNode }) {
  return <td className="whitespace-nowrap px-4 py-3 text-sm text-gray-900">{children}</td>;
}
