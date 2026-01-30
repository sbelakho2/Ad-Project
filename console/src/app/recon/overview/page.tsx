'use client';

import { useEffect, useMemo, useState } from 'react';
import type { ReconOverviewResult } from '@/lib/vra';
import { getOverview } from '@/lib/vra';
import { usePlanAccess } from '@/lib/usePlanAccess';

export default function VraOverviewPage() {
  const { allowed, isLoading: planLoading, plan, error: planError } = usePlanAccess(['growth', 'scale', 'enterprise']);
  const [data, setData] = useState<ReconOverviewResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (planLoading || !allowed) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    getOverview({})
      .then((d) => {
        if (!cancelled) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => !cancelled && setError(String(e?.message || e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, []);

  const topNetworks = useMemo(() => {
    const list = data?.byNetwork || [];
    return [...list].sort((a, b) => b.paid - a.paid).slice(0, 6);
  }, [data]);

  // Compute default window with 24h intraday buffer (excludes today to avoid false Underpay flags
  // from partially ingested network reports)
  const { defaultFrom, defaultTo, bufferNote } = useMemo(() => {
    const now = new Date();
    // End at midnight UTC yesterday (24h buffer)
    const toDate = new Date(now.getTime() - 24 * 60 * 60 * 1000);
    toDate.setUTCHours(0, 0, 0, 0);
    // Start 7 days before that
    const fromDate = new Date(toDate.getTime() - 7 * 24 * 60 * 60 * 1000);
    
    const formatDate = (d: Date) => d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
    
    return {
      defaultFrom: fromDate.toISOString(),
      defaultTo: toDate.toISOString(),
      bufferNote: `${formatDate(fromDate)} – ${formatDate(new Date(toDate.getTime() - 1))} (excludes today for data completeness)`,
    };
  }, []);

  return (
    <div className="p-6 space-y-6">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
        <h1 className="text-xl font-semibold">Reconciliation Overview</h1>
        <div className="text-xs text-gray-500 bg-gray-50 px-2 py-1 rounded-md">
          {bufferNote}
        </div>
      </div>

      {planLoading && <p className="text-sm text-gray-500">Checking your plan…</p>}
      {!planLoading && !allowed && (
        <div className="rounded-md border border-amber-200 bg-amber-50 p-4" role="alert">
          <p className="text-sm font-semibold text-amber-900">Upgrade to access VRA reconciliation</p>
          <p className="text-sm text-amber-800 mt-1">Starter is limited to core dashboards. VRA reconciliation and transparency logs are available on Growth and above. Current plan: {plan || 'unknown'}.</p>
          <div className="mt-3 flex gap-2 flex-wrap">
            <a href="/settings/billing/settings" className="px-3 py-2 text-sm font-semibold text-primary-700 border border-primary-200 rounded-lg hover:bg-primary-50">Open Billing</a>
            <a href="mailto:billing@apexmediation.ee" className="px-3 py-2 text-sm font-semibold text-white bg-primary-600 rounded-lg hover:bg-primary-700">Talk to billing</a>
          </div>
        </div>
      )}
      {planError && !planLoading && <p className="text-sm text-red-600">{planError}</p>}

      {loading && <p className="text-sm text-gray-500">Loading…</p>}
      {error && <p className="text-sm text-red-600">{error}</p>}

      {(!loading && !error && allowed) && (
        <>
          <section className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {topNetworks.length === 0 ? (
              <div className="col-span-full rounded-xl border border-gray-100 p-6 text-sm text-gray-600 bg-white shadow-sm">
                No per-network data yet. Once statements and expected values are populated, cards will appear here.
              </div>
            ) : (
              topNetworks.map((n) => {
                const variance = n.expected - n.paid;
                const variancePercent = n.expected > 0 ? (variance / n.expected) * 100 : 0;
                return (
                  <div key={n.network} className="rounded-xl border border-gray-100 p-5 bg-white shadow-sm hover:shadow-md transition-shadow">
                    <div className="flex items-center justify-between">
                      <div>
                        <div className="text-xs uppercase text-gray-400 tracking-wide">Network</div>
                        <div className="text-lg font-semibold text-gray-900">{n.network}</div>
                      </div>
                      {Math.abs(variancePercent) > 3 && (
                        <span className={`text-xs font-medium px-2 py-1 rounded-full ${
                          variancePercent > 0 ? 'bg-red-100 text-red-700' : 'bg-green-100 text-green-700'
                        }`}>
                          {variancePercent > 0 ? '+' : ''}{variancePercent.toFixed(1)}% var
                        </span>
                      )}
                    </div>
                    {/* Mobile: 2 column, larger screens: 3 columns */}
                    <div className="mt-4 grid grid-cols-2 sm:grid-cols-3 gap-3 text-sm">
                      <div className="rounded-lg bg-gray-50 p-2.5">
                        <div className="text-xs text-gray-500">Impressions</div>
                        <div className="font-semibold text-gray-900 truncate">{n.impressions.toLocaleString()}</div>
                      </div>
                      <div className="rounded-lg bg-gray-50 p-2.5">
                        <div className="text-xs text-gray-500">Paid</div>
                        <div className="font-semibold text-gray-900 truncate">${n.paid.toLocaleString(undefined, { maximumFractionDigits: 0 })}</div>
                      </div>
                      <div className="rounded-lg bg-gray-50 p-2.5 col-span-2 sm:col-span-1">
                        <div className="text-xs text-gray-500">Expected</div>
                        <div className="font-semibold text-gray-900 truncate">${n.expected.toLocaleString(undefined, { maximumFractionDigits: 0 })}</div>
                      </div>
                    </div>
                    <div className="mt-4">
                      <a
                        className="inline-flex items-center gap-1.5 text-xs font-medium text-primary-600 hover:text-primary-700 rounded-lg border border-primary-200 px-3 py-1.5 hover:bg-primary-50 transition-colors"
                        href={`/recon/deltas?from=${encodeURIComponent(defaultFrom)}&to=${encodeURIComponent(defaultTo)}&network=${encodeURIComponent(n.network)}`}
                      >
                        View deltas
                        <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                        </svg>
                      </a>
                    </div>
                  </div>
                );
              })
            )}
          </section>

          <section className="rounded-xl border border-gray-100 p-5 bg-white shadow-sm">
            <div className="flex items-center justify-between">
              <h2 className="text-base font-semibold text-gray-900">Totals</h2>
            </div>
            <div className="mt-4 grid grid-cols-1 sm:grid-cols-3 gap-4 text-sm">
              <div className="rounded-lg bg-gray-50 p-3">
                <div className="text-xs text-gray-500 uppercase tracking-wide">Coverage</div>
                <div className="mt-1 text-lg font-semibold text-gray-900">{(data?.coveragePercent ?? 0).toFixed(1)}%</div>
              </div>
              <div className="rounded-lg bg-gray-50 p-3">
                <div className="text-xs text-gray-500 uppercase tracking-wide">Paid (USD)</div>
                <div className="mt-1 text-lg font-semibold text-gray-900">${(data?.totals.paid ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
              </div>
              <div className="rounded-lg bg-gray-50 p-3">
                <div className="text-xs text-gray-500 uppercase tracking-wide">Expected (USD)</div>
                <div className="mt-1 text-lg font-semibold text-gray-900">${(data?.totals.expected ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
              </div>
            </div>
          </section>

          <section className="rounded-xl border border-gray-100 p-5 bg-white shadow-sm">
            <h2 className="text-base font-semibold text-gray-900 mb-4">By Breakdown (network/format/country)</h2>
            {/* Mobile card view */}
            <div className="block lg:hidden space-y-3">
              {(data?.byBreakdown || []).length === 0 ? (
                <p className="text-sm text-gray-500">No breakdown data available.</p>
              ) : (
                (data?.byBreakdown || []).map((r) => (
                  <div key={`${r.network}-${r.format}-${r.country}`} className="rounded-lg border border-gray-100 p-4 bg-gray-50">
                    <div className="flex items-center justify-between mb-2">
                      <span className="font-medium text-gray-900">{r.network}</span>
                      <span className="text-xs text-gray-500">{r.format} · {r.country}</span>
                    </div>
                    <div className="grid grid-cols-2 gap-2 text-sm">
                      <div>
                        <span className="text-xs text-gray-500">Impressions</span>
                        <p className="font-medium">{r.impressions.toLocaleString()}</p>
                      </div>
                      <div>
                        <span className="text-xs text-gray-500">Paid</span>
                        <p className="font-medium">${r.paid.toLocaleString(undefined, { maximumFractionDigits: 2 })}</p>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
            {/* Desktop table view */}
            <div className="hidden lg:block overflow-x-auto">
              <table className="min-w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-500 border-b border-gray-100">
                    <th className="py-3 pr-4 font-semibold">Network</th>
                    <th className="py-3 pr-4 font-semibold">Format</th>
                    <th className="py-3 pr-4 font-semibold">Country</th>
                    <th className="py-3 pr-4 font-semibold text-right">Impressions</th>
                    <th className="py-3 pr-4 font-semibold text-right">Paid (USD)</th>
                  </tr>
                </thead>
                <tbody>
                  {(data?.byBreakdown || []).map((r) => (
                    <tr key={`${r.network}-${r.format}-${r.country}`} className="border-b border-gray-50 hover:bg-gray-50 transition-colors">
                      <td className="py-3 pr-4 font-medium text-gray-900">{r.network}</td>
                      <td className="py-3 pr-4 text-gray-600">{r.format}</td>
                      <td className="py-3 pr-4 text-gray-600">{r.country}</td>
                      <td className="py-3 pr-4 text-right tabular-nums">{r.impressions.toLocaleString()}</td>
                      <td className="py-3 pr-4 text-right tabular-nums font-medium">${r.paid.toLocaleString(undefined, { maximumFractionDigits: 2 })}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
