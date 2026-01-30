'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReconDeltaItem, ReconDeltaQuery } from '@/lib/vra';
import { buildDeltasCsvUrl, getDeltas } from '@/lib/vra';
import { usePlanAccess } from '@/lib/usePlanAccess';

const KIND_OPTIONS: Array<{ label: string; value: ReconDeltaItem['kind'] | '' }> = [
  { label: 'All kinds', value: '' },
  { label: 'Underpay', value: 'underpay' },
  { label: 'Missing', value: 'missing' },
  { label: 'Viewability gap', value: 'viewability_gap' },
  { label: 'IVT outlier', value: 'ivt_outlier' },
  { label: 'FX mismatch', value: 'fx_mismatch' },
  { label: 'Timing lag', value: 'timing_lag' },
];

function useQueryState() {
  const [query, setQuery] = useState<ReconDeltaQuery>(() => {
    if (typeof window === 'undefined') {
      return { page_size: 50 };
    }
    const u = new URL(window.location.href);
    const q: ReconDeltaQuery = {};
    u.searchParams.forEach((v, k) => ((q as any)[k] = v));
    if (q.page) q.page = Number(q.page);
    if (q.page_size) q.page_size = Number(q.page_size);
    if (q.min_conf != null) q.min_conf = Number(q.min_conf);
    return { page_size: 50, ...q };
  });

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const u = new URL(window.location.href);
    // Reset existing params
    u.search = '';
    for (const [k, v] of Object.entries(query)) {
      if (v == null || v === '') continue;
      u.searchParams.set(k, String(v));
    }
    window.history.replaceState({}, '', u.toString());
  }, [query]);

  return { query, setQuery } as const;
}

export default function VraDeltasPage() {
  const { allowed, isLoading: planLoading, plan, error: planError } = usePlanAccess(['growth', 'scale', 'enterprise']);
  const { query, setQuery } = useQueryState();
  const [items, setItems] = useState<ReconDeltaItem[]>([]);
  const [page, setPage] = useState<number>(query.page || 1);
  const [pageSize, setPageSize] = useState<number>(query.page_size || 50);
  const [total, setTotal] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    if (!allowed) {
      setItems([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    getDeltas({ ...query, page, page_size: pageSize })
      .then((res) => {
        setItems(res.items);
        setTotal(res.total);
      })
      .catch((e) => setError(String(e?.message || e)))
      .finally(() => setLoading(false));
  }, [allowed, query, page, pageSize]);

  useEffect(() => {
    load();
  }, [load]);

  const csvUrl = useMemo(() => buildDeltasCsvUrl({ ...query, page, page_size: pageSize }), [query, page, pageSize]);

  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  const copyDeepLink = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(window.location.href);
    } catch {
      // no-op if clipboard is unavailable
    }
  }, []);

  function ConfidenceBadge({ value }: { value: number }) {
    const v = Number.isFinite(value) ? value : 0;
    let label = 'Low';
    let cls = 'bg-red-100 text-red-800 border-red-200';
    let description = 'Unlikely match or significant discrepancy.';

    if (v >= 0.8) { 
      label = 'High'; 
      cls = 'bg-green-100 text-green-800 border-green-200';
      description = 'Very strong match based on multiple keys (Exact ID or high proximity).';
    }
    else if (v >= 0.5) { 
      label = 'Medium'; 
      cls = 'bg-yellow-100 text-yellow-800 border-yellow-200';
      description = 'Reasonable match based on time and amount proximity.';
    }

    return (
      <span 
        aria-label={`confidence ${label}`} 
        title={`${label} Confidence (${(v * 100).toFixed(0)}%): ${description}`} 
        className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs cursor-help ${cls}`}
      >
        {label}
      </span>
    );
  }

  const kindSelectId = 'recon-kind-filter';
  const minConfidenceId = 'recon-min-confidence';
  const fromId = 'recon-from';
  const toId = 'recon-to';
  const pageSizeId = 'recon-page-size';

  return (
    <div className="p-6 space-y-6">
      <h1 className="text-xl font-semibold">Reconciliation Deltas</h1>

      {planLoading && <p className="text-sm text-gray-500">Checking your plan…</p>}
      {!planLoading && !allowed && (
        <div className="rounded-md border border-amber-200 bg-amber-50 p-4" role="alert">
          <p className="text-sm font-semibold text-amber-900">Upgrade to access reconciliation deltas</p>
          <p className="text-sm text-amber-800 mt-1">Starter only includes core dashboards. Deltas and VRA exports are available on Growth and above. Current plan: {plan || 'unknown'}.</p>
          <div className="mt-3 flex gap-2 flex-wrap">
            <a href="/settings/billing/settings" className="px-3 py-2 text-sm font-semibold text-primary-700 border border-primary-200 rounded-lg hover:bg-primary-50">Open Billing</a>
            <a href="mailto:billing@apexmediation.ee" className="px-3 py-2 text-sm font-semibold text-white bg-primary-600 rounded-lg hover:bg-primary-700">Talk to billing</a>
          </div>
        </div>
      )}
      {planError && !planLoading && <p className="text-sm text-red-600">{planError}</p>}

      {allowed && (
      <section className="rounded-md border p-4 bg-white">
        <form
          className="grid grid-cols-1 md:grid-cols-5 gap-4 items-end"
          onSubmit={(e) => {
            e.preventDefault();
            setPage(1);
            load();
          }}
        >
          <div>
            <label htmlFor={kindSelectId} className="block text-xs text-gray-600 mb-1">Kind</label>
            <select
              id={kindSelectId}
              className="w-full rounded border px-2 py-1"
              value={(query.kind as string) || ''}
              onChange={(e) => setQuery((q) => ({ ...q, kind: (e.target.value as any) || undefined }))}
            >
              {KIND_OPTIONS.map((o) => (
                <option key={o.label} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label htmlFor={minConfidenceId} className="block text-xs text-gray-600 mb-1">Min confidence</label>
            <input
              id={minConfidenceId}
              type="number"
              step="0.05"
              min={0}
              max={1}
              value={query.min_conf ?? ''}
              onChange={(e) => setQuery((q) => ({ ...q, min_conf: e.target.value === '' ? undefined : Number(e.target.value) }))}
              className="w-full rounded border px-2 py-1"
              placeholder="0.5"
            />
          </div>
          <div>
            <label htmlFor={fromId} className="block text-xs text-gray-600 mb-1">From</label>
            <input
              id={fromId}
              type="datetime-local"
              value={query.from ? toLocalInputValue(query.from) : ''}
              onChange={(e) => setQuery((q) => ({ ...q, from: e.target.value ? new Date(e.target.value).toISOString() : undefined }))}
              className="w-full rounded border px-2 py-1"
            />
          </div>
          <div>
            <label htmlFor={toId} className="block text-xs text-gray-600 mb-1">To</label>
            <input
              id={toId}
              type="datetime-local"
              value={query.to ? toLocalInputValue(query.to) : ''}
              onChange={(e) => setQuery((q) => ({ ...q, to: e.target.value ? new Date(e.target.value).toISOString() : undefined }))}
              className="w-full rounded border px-2 py-1"
            />
          </div>
          <div className="flex items-center gap-2">
            <button type="submit" className="rounded bg-black text-white px-3 py-2 text-sm">Apply</button>
            <a href={csvUrl} className="rounded border px-3 py-2 text-sm" download>
              Download CSV
            </a>
            <button
              type="button"
              className="rounded border px-3 py-2 text-sm"
              onClick={copyDeepLink}
              aria-label="Copy deep link to current filters"
              title="Copy link to current filters"
            >
              Copy link
            </button>
          </div>
        </form>
      </section>
      )}

      {allowed && (
      <section className="rounded-md border bg-white">
        <div className="flex items-center justify-between p-4">
          <div className="text-sm text-gray-600">Total: {total.toLocaleString()}</div>
          <div className="flex items-center gap-2">
            <label htmlFor={pageSizeId} className="text-xs text-gray-500">Page size</label>
            <select
              id={pageSizeId}
              className="rounded border px-2 py-1 text-sm"
              value={pageSize}
              onChange={(e) => {
                setPageSize(Number(e.target.value));
                setPage(1);
              }}
            >
              {[25, 50, 100, 200, 500].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </div>
        </div>
        <div className="overflow-auto">
          {/* Legend */}
          <div className="px-4 pb-2 text-xs text-gray-600 flex items-center gap-3">
            <span className="inline-flex items-center gap-1"><span className="inline-block w-3 h-3 rounded-full bg-green-200 border border-green-300" /> High ≥ 0.80</span>
            <span className="inline-flex items-center gap-1"><span className="inline-block w-3 h-3 rounded-full bg-yellow-200 border border-yellow-300" /> Medium 0.50–0.79</span>
            <span className="inline-flex items-center gap-1"><span className="inline-block w-3 h-3 rounded-full bg-red-200 border border-red-300" /> Low &lt; 0.50</span>
          </div>
          <table className="min-w-full text-sm">
            <thead>
              <tr className="text-left text-gray-500 sticky top-0 bg-white">
                <th className="py-2 px-4">Kind</th>
                <th className="py-2 px-4">Amount</th>
                <th className="py-2 px-4">Currency</th>
                <th className="py-2 px-4">Reason</th>
                <th className="py-2 px-4">Window</th>
                <th className="py-2 px-4">Evidence</th>
                <th className="py-2 px-4">Conf</th>
              </tr>
            </thead>
            <tbody>
              {loading && (
                <tr>
                  <td className="py-3 px-4" colSpan={7}>
                    <span className="text-gray-500">Loading…</span>
                  </td>
                </tr>
              )}
              {error && !loading && (
                <tr>
                  <td className="py-3 px-4" colSpan={7}>
                    <span className="text-red-600">{error}</span>
                  </td>
                </tr>
              )}
              {!loading && !error && items.length === 0 && (
                <tr>
                  <td className="py-3 px-4" colSpan={7}>
                    <span className="text-gray-600">No deltas for the selected filters.</span>
                  </td>
                </tr>) }
              {!loading && !error && items.map((r) => (
                <tr key={r.evidenceId} className="border-t align-top">
                  <td className="py-2 px-4">{r.kind}</td>
                  <td className="py-2 px-4">${r.amount.toLocaleString(undefined, { maximumFractionDigits: 2 })}</td>
                  <td className="py-2 px-4">{r.currency}</td>
                  <td className="py-2 px-4">
                    <Reason code={r.reasonCode} />
                  </td>
                  <td className="py-2 px-4 whitespace-nowrap">
                    <div>{r.windowStart}</div>
                    <div className="text-xs text-gray-500">{r.windowEnd}</div>
                  </td>
                  <td className="py-2 px-4">{r.evidenceId}</td>
                  <td className="py-2 px-4 flex items-center gap-2">
                    <span className="text-xs text-gray-600">{r.confidence.toFixed(2)}</span>
                    <ConfidenceBadge value={r.confidence} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="flex items-center justify-between p-4 border-t">
          <div className="text-sm text-gray-600">
            Page {page} of {totalPages}
          </div>
          <div className="flex gap-2">
            <button
              className="rounded border px-3 py-1 text-sm disabled:opacity-50"
              disabled={page <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
            >
              Prev
            </button>
            <button
              className="rounded border px-3 py-1 text-sm disabled:opacity-50"
              disabled={page >= totalPages}
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
            >
              Next
            </button>
          </div>
        </div>
      </section>
      )}
    </div>
  );
}

function toLocalInputValue(iso: string): string {
  try {
    const d = new Date(iso);
    const pad = (n: number) => String(n).padStart(2, '0');
    const yyyy = d.getFullYear();
    const mm = pad(d.getMonth() + 1);
    const dd = pad(d.getDate());
    const hh = pad(d.getHours());
    const mi = pad(d.getMinutes());
    return `${yyyy}-${mm}-${dd}T${hh}:${mi}`;
  } catch {
    return '';
  }
}

function Reason({ code }: { code: string }) {
  // Redaction-safe display: do not render commas/newlines; truncate long strings; rely on backend redaction too.
  const safe = String(code || '')
    .replace(/[\r\n]+/g, ' ')
    .replace(/,/g, ' ')
    .slice(0, 240);
  return <span title={safe} className="whitespace-pre-wrap break-words">{safe}</span>;
}
