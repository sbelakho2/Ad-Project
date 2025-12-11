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

  const defaultFrom = useMemo(() => new Date(Date.now() - 7 * 24 * 3600 * 1000).toISOString(), []);
  const defaultTo = useMemo(() => new Date().toISOString(), []);

  return (
    <div className="p-6 space-y-6">
      <h1 className="text-xl font-semibold">Reconciliation Overview</h1>

      {planLoading && <p className="text-sm text-gray-500">Checking your plan…</p>}
      {!planLoading && !allowed && (
        <div className="rounded-md border border-amber-200 bg-amber-50 p-4" role="alert">
          <p className="text-sm font-semibold text-amber-900">Upgrade to access VRA reconciliation</p>
          <p className="text-sm text-amber-800 mt-1">Starter is limited to core dashboards. VRA reconciliation and transparency logs are available on Growth and above. Current plan: {plan || 'unknown'}.</p>
          <div className="mt-3 flex gap-2 flex-wrap">
            <a href="/billing/settings" className="px-3 py-2 text-sm font-semibold text-primary-700 border border-primary-200 rounded-lg hover:bg-primary-50">Open Billing</a>
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
              <div className="col-span-full rounded-md border p-4 text-sm text-gray-600">
                No per-network data yet. Once statements and expected values are populated, cards will appear here.
              </div>
            ) : (
              topNetworks.map((n) => (
                <div key={n.network} className="rounded-md border p-4 bg-white">
                  <div className="text-xs uppercase text-gray-500">Network</div>
                  <div className="text-lg font-medium">{n.network}</div>
                  <div className="mt-2 grid grid-cols-3 gap-2 text-sm">
                    <div>
                      <div className="text-xs text-gray-500">Impressions</div>
                      <div className="font-medium">{n.impressions.toLocaleString()}</div>
                    </div>
                    <div>
                      <div className="text-xs text-gray-500">Paid (USD)</div>
                      <div className="font-medium">${n.paid.toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
                    </div>
                    <div>
                      <div className="text-xs text-gray-500">Expected</div>
                      <div className="font-medium">${n.expected.toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
                    </div>
                  </div>
                  <div className="mt-3">
                    <a
                      className="inline-flex items-center text-xs rounded border px-2 py-1 hover:bg-gray-50"
                      // Deep-link to Deltas with the current default window
                      href={`/recon/deltas?from=${encodeURIComponent(defaultFrom)}&to=${encodeURIComponent(defaultTo)}`}
                    >
                      View recent deltas
                    </a>
                  </div>
                </div>
              ))
            )}
          </section>

          <section className="rounded-md border p-4 bg-white">
            <div className="flex items-center justify-between">
              <h2 className="text-base font-semibold">Totals</h2>
            </div>
            <div className="mt-3 grid grid-cols-1 sm:grid-cols-3 gap-4 text-sm">
              <div>
                <div className="text-xs text-gray-500">Coverage</div>
                <div className="font-medium">{(data?.coveragePercent ?? 0).toFixed(1)}%</div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Paid (USD)</div>
                <div className="font-medium">${(data?.totals.paid ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Expected (USD)</div>
                <div className="font-medium">${(data?.totals.expected ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
              </div>
            </div>
          </section>

          <section className="rounded-md border p-4 bg-white">
            <h2 className="text-base font-semibold mb-2">By Breakdown (network/format/country)</h2>
            <div className="overflow-auto">
              <table className="min-w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-500">
                    <th className="py-2 pr-4">Network</th>
                    <th className="py-2 pr-4">Format</th>
                    <th className="py-2 pr-4">Country</th>
                    <th className="py-2 pr-4">Impressions</th>
                    <th className="py-2 pr-4">Paid (USD)</th>
                  </tr>
                </thead>
                <tbody>
                  {(data?.byBreakdown || []).map((r) => (
                    <tr key={`${r.network}-${r.format}-${r.country}`} className="border-t">
                      <td className="py-2 pr-4">{r.network}</td>
                      <td className="py-2 pr-4">{r.format}</td>
                      <td className="py-2 pr-4">{r.country}</td>
                      <td className="py-2 pr-4">{r.impressions.toLocaleString()}</td>
                      <td className="py-2 pr-4">${r.paid.toLocaleString(undefined, { maximumFractionDigits: 2 })}</td>
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
