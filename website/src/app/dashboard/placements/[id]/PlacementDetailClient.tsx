'use client';

import React, { useEffect, useMemo, useState } from 'react';
import Breadcrumbs, { buildBreadcrumbsFromPath } from '@/components/ui/Breadcrumbs';
import { api } from '@/lib/api';
import { usePathname, useRouter } from 'next/navigation';

type Placement = {
  id: string;
  name: string;
  format: 'banner' | 'interstitial' | 'rewarded' | 'native';
  app: string;
  status: 'active' | 'inactive';
  impressions?: number;
  revenueCents?: number;
  ecpmCents?: number;
  fillRate?: number; // 0..1 or 0..100
  ctr?: number; // 0..1 or 0..100
};

function normalizePlacement(p: any): Placement {
  const fill = Number(p.fillRate ?? 0);
  const ctr = Number(p.ctr ?? 0);
  return {
    id: String(p.id),
    name: String(p.name || p.title || 'Untitled'),
    format: (p.format || 'banner') as Placement['format'],
    app: String(p.appName || p.app || '—'),
    status: (p.status === 'active' ? 'active' : 'inactive') as Placement['status'],
    impressions: Math.max(0, Number(p.impressions || 0)),
    revenueCents: Math.max(0, Number(p.revenueCents || 0)),
    ecpmCents: Math.max(0, Number(p.ecpmCents || 0)),
    fillRate: Math.max(0, Math.min(100, fill <= 1 ? fill * 100 : fill)),
    ctr: Math.max(0, Math.min(100, ctr <= 1 ? ctr * 100 : ctr)),
  };
}

export default function PlacementDetailClient({ id }: { id: string }) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [placement, setPlacement] = useState<Placement | null>(null);
  const router = useRouter();
  const pathname = usePathname();

  const crumbs = useMemo(() => buildBreadcrumbsFromPath(pathname || '/'), [pathname]);
  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const curFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'currency', currency }), [currency]);

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    setLoading(true);
    setError(null);
    (async () => {
      const res = await api.get(`/placements/${encodeURIComponent(id)}`, { signal: controller?.signal });
      if (!alive) return;
      if (!res.success || !res.data) {
        setError(res.error || 'Placement not found');
        setLoading(false);
        return;
      }
      setPlacement(normalizePlacement(res.data));
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
  }, [id]);

  function getCsrf(): string | undefined {
    try {
      const m = document.cookie.match(/(?:^|; )csrf_token=([^;]+)/);
      return m && m[1] ? decodeURIComponent(m[1]) : undefined;
    } catch {
      return undefined;
    }
  }

  async function toggleActive() {
    if (!placement) return;
    const next = placement.status === 'active' ? 'inactive' : 'active';
    const res = await api.put(
      `/placements/${placement.id}`,
      { status: next },
      { headers: { ...(getCsrf() ? { 'X-CSRF-Token': getCsrf()! } : {}) } }
    );
    if (!res.success) {
      setError(res.error || 'Failed to update status');
      return;
    }
    setPlacement({ ...placement, status: next });
  }

  async function archive() {
    if (!placement) return;
    if (!confirm('Archive this placement?')) return;
    const res = await api.delete(`/placements/${placement.id}`);
    if (!res.success) {
      setError(res.error || 'Failed to archive');
      return;
    }
    router.push('/dashboard/placements');
  }

  return (
    <div className="p-6 space-y-6">
      <Breadcrumbs items={crumbs} />
      <h1 className="text-h2-sm font-bold uppercase text-gray-900 tracking-tight">Placement Details</h1>
      {loading ? (
        <div className="space-y-3" aria-busy="true">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="h-16 bg-gray-100 animate-pulse rounded" />
          ))}
        </div>
      ) : error ? (
        <div role="alert" aria-live="polite" className="rounded-md border border-red-200 bg-red-50 p-4 text-sm text-red-800">{error}</div>
      ) : !placement ? (
        <div className="text-sm text-gray-600">This placement could not be found.</div>
      ) : (
        <div className="space-y-6">
          <div className="card p-6">
            <div className="flex items-start justify-between">
              <div>
                <p className="text-sm text-gray-600">Name</p>
                <h2 className="text-2xl font-bold text-gray-900">{placement.name}</h2>
                <div className="mt-2 text-sm text-gray-600 flex gap-3">
                  <span>App: <strong className="text-gray-900">{placement.app}</strong></span>
                  <span>Format: <strong className="text-gray-900">{placement.format.toUpperCase()}</strong></span>
                  <span>Status: <strong className={placement.status === 'active' ? 'text-green-600' : 'text-gray-600'}>{placement.status}</strong></span>
                </div>
              </div>
              <div className="flex items-center gap-3">
                <button onClick={toggleActive} className="btn-secondary text-sm px-4">
                  {placement.status === 'active' ? 'Deactivate' : 'Activate'}
                </button>
                <button onClick={archive} className="px-4 py-2 text-sm rounded text-white bg-[rgb(var(--danger))] hover:opacity-90">
                  Archive
                </button>
              </div>
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-4 gap-6">
            <Metric label="Impressions" value={(placement.impressions ?? 0).toLocaleString()} />
            <Metric label="Revenue" value={curFmt.format(Math.max(0, (placement.revenueCents ?? 0) / 100))} />
            <Metric label="eCPM" value={curFmt.format(Math.max(0, (placement.ecpmCents ?? 0) / 100))} />
            <Metric label="Fill Rate" value={`${Math.max(0, Math.min(100, placement.fillRate ?? 0)).toFixed(1)}%`} />
          </div>

          <div className="card p-6">
            <h3 className="text-gray-900 font-bold uppercase text-lg mb-4 border-b-2 border-gray-200 pb-2">Next steps</h3>
            <ul className="list-disc list-inside text-sm text-gray-700 space-y-2">
              <li>Review adapter configuration for this placement in Settings.</li>
              <li>Verify live traffic in Observability → Metrics and Debugger.</li>
              <li>Use Transparency tools to validate auction integrity for recent requests.</li>
            </ul>
          </div>
        </div>
      )}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="card p-6">
      <p className="text-xs text-gray-600 uppercase">{label}</p>
      <p className="text-lg font-bold text-gray-900">{value}</p>
    </div>
  );
}
