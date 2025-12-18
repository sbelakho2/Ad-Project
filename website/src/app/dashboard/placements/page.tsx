"use client";
export const dynamic = 'force-dynamic';

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Placements Page"
// Ad placements management with performance heatmap and format configuration

import { PlusCircleIcon } from '@heroicons/react/24/outline';
import { Suspense, useEffect, useMemo, useState } from 'react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { api } from '@/lib/api';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

// Placement shape normalized for UI
interface Placement {
  id: string;
  name: string;
  format: 'banner' | 'interstitial' | 'rewarded' | 'native';
  app: string;
  status: 'active' | 'inactive';
  impressions: number;
  revenue: number; // major units
  ecpm: number; // major units
  fillRate: number; // percent 0..100
  ctr: number; // percent 0..100
  // Optional backend configuration document (echoed by API)
  config?: any;
}

function PlacementsPageInner() {
  const router = useRouter();
  const search = useSearchParams();
  const pathname = usePathname();

  // Read initial state from the URL (persist filters/pagination)
  const initialFormat = (search.get('format') as any) || 'all';
  const [selectedFormat, setSelectedFormat] = useState<'all' | 'banner' | 'interstitial' | 'rewarded' | 'native'>(initialFormat);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [placements, setPlacements] = useState<Placement[]>([]);
  const [page, setPage] = useState(Number(search.get('page') || 1));
  const [pageSize] = useState(20);
  const [total, setTotal] = useState(0);

  const currency = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
  const curFmt = useMemo(() => new Intl.NumberFormat(undefined, { style: 'currency', currency }), [currency]);

  // Keep URL in sync with UI state
  useEffect(() => {
    const params = new URLSearchParams(search?.toString());
    params.set('page', String(page));
    params.set('pageSize', String(pageSize));
    params.set('format', selectedFormat);
    // Avoid router spam: only push when something changed
    const next = `${pathname}?${params.toString()}`;
    const curr = `${pathname}?${search?.toString()}`;
    if (next !== curr) router.replace(next);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, pageSize, selectedFormat]);

  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    setLoading(true);
    setError(null);
    (async () => {
      const params = new URLSearchParams();
      params.set('page', String(page));
      params.set('pageSize', String(pageSize));
      if (selectedFormat !== 'all') params.set('format', selectedFormat);
      const res = await api.get<{ items: any[]; total: number }>(`/placements?${params.toString()}` , { signal: controller?.signal });
      if (!alive) return;
      if (!res.success || !res.data) {
        setError(res.error || 'Failed to load placements');
        setLoading(false);
        return;
      }
      const mapped: Placement[] = (res.data.items || []).map((p: any) => ({
        id: String(p.id),
        name: String(p.name || p.title || 'Untitled'),
        format: (p.format || 'banner') as Placement['format'],
        app: String(p.appName || p.app || '—'),
        status: (p.status === 'active' ? 'active' : 'inactive') as Placement['status'],
        impressions: Math.max(0, Number(p.impressions || 0)),
        revenue: Math.max(0, Number(p.revenueCents || 0) / 100),
        ecpm: Math.max(0, Number(p.ecpmCents || 0) / 100),
        fillRate: Math.max(0, Math.min(100, Number(p.fillRate || 0) * (p.fillRate <= 1 ? 100 : 1))),
        ctr: Math.max(0, Math.min(100, Number(p.ctr || 0) * (p.ctr <= 1 ? 100 : 1))),
        config: p.config || {},
      }));
      setPlacements(mapped);
      setTotal(Math.max(0, Number(res.data.total || mapped.length)));
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
  }, [page, pageSize, selectedFormat]);

  const [modalState, setModalState] = useState<
    | { type: null }
    | { type: 'create' }
    | { type: 'configure' | 'view-details'; placement: Placement }
  >({ type: null });

  const openModal = (type: 'create' | 'configure' | 'view-details', placement?: Placement) => {
    if (type === 'create') {
      setModalState({ type });
    } else if (placement) {
      setModalState({ type, placement });
    }
  };

  const closeModal = () => setModalState({ type: null });

  // Helpers to discover CSRF token from cookie (best-effort)
  function getCsrf(): string | undefined {
    try {
      const m = document.cookie.match(/(?:^|; )csrf_token=([^;]+)/);
      return m && m[1] ? decodeURIComponent(m[1]) : undefined;
    } catch {
      return undefined;
    }
  }

  // API actions — Create / Update / Archive with basic CSRF header
  async function createPlacement(input: { name: string; app: string; format: Placement['format'] }) {
    const res = await api.post('/placements', input, {
      headers: {
        ...(getCsrf() ? { 'X-CSRF-Token': getCsrf()! } : {}),
      },
    });
    if (!res.success) throw new Error(res.error || 'Failed to create placement');
    // Refresh list
    setPage(1);
  }

  async function updatePlacement(id: string, patch: Partial<Placement>) {
    const res = await api.patch(`/placements/${id}`, patch, {
      headers: {
        ...(getCsrf() ? { 'X-CSRF-Token': getCsrf()! } : {}),
      },
    });
    if (!res.success) throw new Error(res.error || 'Failed to update placement');
  }

  async function archivePlacement(id: string) {
    const res = await api.delete(`/placements/${id}`, {
      headers: {
        ...(getCsrf() ? { 'X-CSRF-Token': getCsrf()! } : {}),
      },
    });
    if (!res.success) throw new Error(res.error || 'Failed to archive placement');
    // Remove from local list optimistically
    setPlacements((prev) => prev.filter((p) => p.id !== id));
  }

  const handleActivatePlacement = async (id: string) => {
    try {
      await updatePlacement(id, { status: 'active' } as any);
      setPlacements((prev) => prev.map((p) => (p.id === id ? { ...p, status: 'active' } : p)));
    } catch (e: any) {
      setError(e?.message || 'Failed to activate placement');
    }
  };

  const filteredPlacements = placements; // server-filtered above; keep for safety

  const totalRevenue = placements.reduce((sum, p) => sum + Math.max(0, p.revenue), 0);
  const totalImpressions = placements.reduce((sum, p) => sum + Math.max(0, p.impressions), 0);
  const avgEcpm = totalImpressions > 0 ? totalRevenue / (totalImpressions / 1000) : 0;
  const activePlacements = placements.filter(p => p.status === 'active').length;

  return (
    <Section>
      <Container className="space-y-6">
      {/* Page Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">
            Ad Placements
          </h1>
          <p className="mt-1 text-sm text-gray-600">
            Manage ad placements and optimize performance across all apps
          </p>
        </div>
        <button
          className="btn-primary flex items-center gap-2"
          onClick={() => openModal('create')}
        >
          <PlusCircleIcon className="w-5 h-5" />
          Create Placement
        </button>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-1 gap-4 md:grid-cols-4">
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">Active Placements</p>
            <p className="text-3xl font-semibold text-gray-900">{activePlacements}</p>
            <p className="mt-1 text-sm text-gray-600">of {placements.length} total</p>
          </div>
        </div>
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">Total Revenue</p>
            <p className="text-3xl font-semibold text-gray-900">{curFmt.format(totalRevenue)}</p>
            <p className="mt-1 text-sm text-gray-600">this week</p>
          </div>
        </div>
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">Avg eCPM</p>
            <p className="text-3xl font-semibold text-gray-900">{curFmt.format(avgEcpm)}</p>
            <p className="mt-1 text-sm text-gray-600">across all placements</p>
          </div>
        </div>
        <div className="card-v2">
          <div className="card-v2-body">
            <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">Impressions</p>
            <p className="text-3xl font-semibold text-gray-900">{totalImpressions.toLocaleString()}</p>
            <p className="mt-1 text-sm text-gray-600">this week</p>
          </div>
        </div>
      </div>

      {/* Format Filter */}
      <div className="card-v2">
        <div className="card-v2-header">
          <h2 className="text-sm font-semibold text-gray-900">Filter by Format</h2>
        </div>
        <div className="card-v2-body">
          <div className="flex flex-wrap gap-3">
            {['all', 'banner', 'interstitial', 'rewarded', 'native'].map((format) => (
              <button
                key={format}
                onClick={() => setSelectedFormat(format as any)}
                className={`rounded-xl px-4 py-2 text-sm font-semibold transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
                  selectedFormat === format
                    ? 'bg-brand-50 text-brand-700 border border-brand-500'
                    : 'bg-white text-gray-700 border border-gray-300 hover:border-brand-500'
                }`}
              >
                {format === 'all' ? '📊 All Formats' :
                 format === 'banner' ? '📱 Banner' :
                 format === 'interstitial' ? '🖼️ Interstitial' :
                 format === 'rewarded' ? '🎁 Rewarded' :
                 '📰 Native'}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* Performance Heatmap */}
      <div className="card-v2">
        <div className="card-v2-header">
          <h2 className="text-sm font-semibold text-gray-900">Format Performance Comparison</h2>
        </div>
        <div className="card-v2-body grid gap-6 md:grid-cols-4">
          <FormatHeatmapCard
            format="Banner"
            icon="📱"
            avgEcpm={10.00}
            avgFillRate={98.9}
            avgCtr={1.05}
            count={2}
          />
          <FormatHeatmapCard
            format="Interstitial"
            icon="🖼️"
            avgEcpm={14.59}
            avgFillRate={96.7}
            avgCtr={3.1}
            count={2}
          />
          <FormatHeatmapCard
            format="Rewarded"
            icon="🎁"
            avgEcpm={22.43}
            avgFillRate={93.5}
            avgCtr={16.7}
            count={3}
          />
          <FormatHeatmapCard
            format="Native"
            icon="📰"
            avgEcpm={0}
            avgFillRate={0}
            avgCtr={0}
            count={1}
            inactive
          />
        </div>
      </div>

      {/* Placements List */}
      <div className="space-y-4">
        <h2 className="text-sm font-semibold text-gray-900">
          {selectedFormat === 'all' ? 'All Placements' : `${selectedFormat} Placements`}
          <span className="text-gray-500 text-sm ml-2">({total})</span>
        </h2>
        {loading ? (
          <div className="space-y-3 minh-table" aria-hidden="true">
            {[...Array(5)].map((_, i) => (
              <div key={i} className="h-20 bg-gray-100 animate-pulse rounded" />
            ))}
          </div>
        ) : error ? (
          <div role="alert" aria-live="polite" className="rounded-md border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
            {error}
          </div>
        ) : filteredPlacements.length === 0 ? (
          <p className="text-sm text-gray-600">No placements found.</p>
        ) : (
          <>
            {filteredPlacements.map((placement) => (
              <PlacementCard
                key={placement.id}
                placement={placement}
                onConfigure={() => openModal('configure', placement)}
                onViewDetails={() => openModal('view-details', placement)}
                onActivate={() => handleActivatePlacement(placement.id)}
              />
            ))}
            <div className="flex items-center justify-end gap-2 pt-2">
              <button
                className="px-3 py-1 text-sm border rounded disabled:opacity-50"
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                disabled={page <= 1}
              >
                Prev
              </button>
              <span className="text-sm" aria-live="polite">Page {page}</span>
              <button
                className="px-3 py-1 text-sm border rounded disabled:opacity-50"
                onClick={() => setPage((p) => p + 1)}
                disabled={placements.length < pageSize}
              >
                Next
              </button>
            </div>
          </>
        )}
      </div>

      {/* Best Practices */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-semibold text-lg mb-4">
          Placement Best Practices
        </h2>
        <div className="grid md:grid-cols-2 gap-6 text-sm text-gray-700">
          <div className="space-y-3">
            <div className="flex items-start gap-2">
              <span className="text-brand-600">✓</span>
              <div>
                <p className="font-semibold mb-1 text-gray-900">Banner Ads</p>
                <p className="text-gray-700">Place at top or bottom of screen. Avoid covering content. Refresh every 30-60 seconds.</p>
              </div>
            </div>
            <div className="flex items-start gap-2">
              <span className="text-brand-600">✓</span>
              <div>
                <p className="font-semibold mb-1 text-gray-900">Interstitial Ads</p>
                <p className="text-gray-700">Show at natural break points (level complete, menu transitions). Limit frequency to once per 2-3 minutes.</p>
              </div>
            </div>
          </div>
          <div className="space-y-3">
            <div className="flex items-start gap-2">
              <span className="text-brand-600">✓</span>
              <div>
                <p className="font-semibold mb-1 text-gray-900">Rewarded Video</p>
                <p className="text-gray-700">Offer clear value (extra lives, hints, coins). Always optional. Show reward before video starts.</p>
              </div>
            </div>
            <div className="flex items-start gap-2">
              <span className="text-brand-600">✓</span>
              <div>
                <p className="font-semibold mb-1 text-gray-900">Native Ads</p>
                <p className="text-gray-700">Match your app&rsquo;s design. Clearly label as &ldquo;Sponsored&rdquo;. Use in content feeds or articles.</p>
              </div>
            </div>
          </div>
        </div>
      </div>
      <PlacementModal
        state={modalState}
        onClose={closeModal}
        onCreate={async (payload) => {
          try {
            await createPlacement(payload);
            closeModal();
          } catch (e: any) {
            setError(e?.message || 'Failed to create placement');
          }
        }}
        onArchive={async (id) => {
          try {
            if (confirm('Archive this placement? You can re-activate later.')) {
              await archivePlacement(id);
              closeModal();
            }
          } catch (e: any) {
            setError(e?.message || 'Failed to archive placement');
          }
        }}
        onSaveConfig={async (id, patch) => {
          try {
            await updatePlacement(id, patch);
            // Update local state best-effort
            setPlacements((prev) => prev.map((p) => (p.id === id ? { ...p, ...patch } as Placement : p)));
            closeModal();
          } catch (e: any) {
            setError(e?.message || 'Failed to save');
          }
        }}
      />
      </Container>
    </Section>
  );
}

export default function PlacementsPage() {
  return (
    <Suspense fallback={<div className="container py-10"><div className="space-y-3 minh-table" aria-hidden="true">{[...Array(5)].map((_, i) => (<div key={i} className="h-20 bg-gray-100 animate-pulse rounded" />))}</div></div>}>
      <PlacementsPageInner />
    </Suspense>
  );
}

interface FormatHeatmapCardProps {
  format: string;
  icon: string;
  avgEcpm: number;
  avgFillRate: number;
  avgCtr: number;
  count: number;
  inactive?: boolean;
}

function FormatHeatmapCard({ format, icon, avgEcpm, avgFillRate, avgCtr, count, inactive }: FormatHeatmapCardProps) {
  return (
    <div className={`border-2 rounded p-4 ${inactive ? 'border-gray-300 bg-gray-50' : 'border-brand-500'}`}>
      <div className="flex items-center gap-2 mb-3">
        <span className="text-2xl">{icon}</span>
        <h3 className="font-bold text-gray-900">{format}</h3>
      </div>
      {inactive ? (
        <p className="text-sm text-gray-600">No active placements</p>
      ) : (
        <div className="space-y-2 text-sm">
          <div className="flex justify-between">
            <span className="text-gray-600">Avg eCPM:</span>
            <span className="font-bold text-gray-900">${avgEcpm.toFixed(2)}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-600">Fill Rate:</span>
            <span className="font-bold text-gray-900">{avgFillRate.toFixed(1)}%</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-600">Avg CTR:</span>
            <span className="font-bold text-gray-900">{avgCtr.toFixed(2)}%</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-600">Count:</span>
            <span className="font-bold text-gray-900">{count}</span>
          </div>
        </div>
      )}
    </div>
  );
}

interface PlacementCardProps {
  placement: Placement;
  onConfigure: () => void;
  onViewDetails: () => void;
  onActivate: () => void;
}

function PlacementCard({ placement, onConfigure, onViewDetails, onActivate }: PlacementCardProps) {
  return (
    <div className="card p-6">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-bold text-gray-900">{placement.name}</h3>
          <p className="text-sm text-gray-600">{placement.app} • {placement.format.toUpperCase()}</p>
        </div>
        <div className={`text-sm font-bold ${placement.status === 'active' ? 'text-success' : 'text-gray-600'}`}>
          {placement.status}
        </div>
      </div>
      <div className="mt-3 grid grid-cols-2 md:grid-cols-4 gap-4">
        <div>
          <p className="text-xs text-gray-600 uppercase">Impressions</p>
          <p className="text-lg font-bold text-gray-900">{placement.impressions.toLocaleString()}</p>
        </div>
        <div>
          <p className="text-xs text-gray-600 uppercase">Revenue</p>
          <p className="text-lg font-bold text-gray-900">${placement.revenue.toFixed(2)}</p>
        </div>
        <div>
          <p className="text-xs text-gray-600 uppercase">eCPM</p>
          <p className="text-lg font-bold text-gray-900">${placement.ecpm.toFixed(2)}</p>
        </div>
        <div>
          <p className="text-xs text-gray-600 uppercase">Fill Rate</p>
          <p className="text-lg font-bold text-gray-900">{placement.fillRate}%</p>
        </div>
      </div>
      {placement.status !== 'active' && (
        <div className="mt-3">
          <button onClick={onActivate} className="px-4 py-2 text-sm font-bold text-white bg-success-green rounded">Activate</button>
        </div>
      )}
      <div className="flex items-center justify-end gap-3 mt-4 pt-4 border-t border-gray-200">
        <button onClick={onConfigure} className="btn-outline px-4 py-2 text-sm">Configure</button>
        <button onClick={onViewDetails} className="btn-primary px-4 py-2 text-sm">View Details</button>
      </div>
    </div>
  );
}

type PlacementModalState =
  | { type: null }
  | { type: 'create' }
  | { type: 'configure' | 'view-details'; placement: Placement };

function PlacementModal({
  state,
  onClose,
  onCreate,
  onSaveConfig,
  onArchive,
}: {
  state: PlacementModalState;
  onClose: () => void;
  onCreate: (payload: { name: string; app: string; format: Placement['format'] }) => Promise<void>;
  onSaveConfig: (id: string, patch: Partial<Placement>) => Promise<void>;
  onArchive: (id: string) => Promise<void>;
}) {
  if (state.type === null) return null;
  const title = state.type === 'create' ? 'Create New Placement' : state.type === 'configure' ? `Configure ${state.placement.name}` : `Placement Insights for ${state.placement.name}`;

  // Accessibility: focus trap + Esc to close
  const [activeTab, setActiveTab] = useState<'overview' | 'targeting' | 'adunits'>('overview');
  // Simple controlled fields to persist on save — maps to backend config schema
  const [geos, setGeos] = useState<string>('');
  const [platforms, setPlatforms] = useState<string>('iOS & Android');
  const [frequencyCap, setFrequencyCap] = useState<number | ''>('');
  const [floorPriceUsd, setFloorPriceUsd] = useState<number | ''>('');
  const [unitIdIos, setUnitIdIos] = useState<string>('');
  const [unitIdAndroid, setUnitIdAndroid] = useState<string>('');

  // Initialize form fields from existing placement config when opening
  useEffect(() => {
    if (state.type === 'configure' || state.type === 'view-details') {
      const cfg = (state.placement as any).config || {};
      const t = cfg.targeting || {};
      const d = cfg.delivery || {};
      const p = cfg.pricing || {};
      const s = cfg.sdk || {};
      const geoArr: string[] = Array.isArray(t.geos) ? t.geos : [];
      setGeos(geoArr.join(', '));
      const plats: string[] = Array.isArray(t.platforms) ? t.platforms : [];
      setPlatforms(plats.length === 1 && plats[0] === 'ios' ? 'iOS only' : plats.length === 1 && plats[0] === 'android' ? 'Android only' : 'iOS & Android');
      const cap = d.frequencyCap?.count;
      setFrequencyCap(typeof cap === 'number' && cap >= 0 ? cap : '');
      const cents = p.floorPriceCents;
      setFloorPriceUsd(typeof cents === 'number' && cents >= 0 ? Math.round(cents) / 100 : '');
      setUnitIdIos(typeof s.unitIdIos === 'string' ? s.unitIdIos : '');
      setUnitIdAndroid(typeof s.unitIdAndroid === 'string' ? s.unitIdAndroid : '');
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.type, (state as any).placement?.id]);

  // Client-side validation state
  const [fieldErrors, setFieldErrors] = useState<{ geos?: string; frequencyCap?: string; floorPriceUsd?: string }>({});

  // Helpers for mapping UI → backend config payload
  const toIso2Array = (csv: string): string[] =>
    csv
      .split(',')
      .map((s) => s.trim().toUpperCase())
      .filter((s) => s.length > 0);

  const mapPlatforms = (label: string): Array<'ios' | 'android' | 'unity' | 'web'> => {
    switch (label) {
      case 'iOS only':
        return ['ios'];
      case 'Android only':
        return ['android'];
      case 'iOS & Android':
      default:
        return ['ios', 'android'];
    }
  };

  const validateTargeting = () => {
    const next: typeof fieldErrors = {};
    const codes = toIso2Array(geos);
    const invalid = codes.find((c) => !/^[A-Z]{2}$/.test(c));
    if (invalid) next.geos = `Invalid country code: ${invalid}`;
    if (frequencyCap !== '' && (typeof frequencyCap !== 'number' || frequencyCap < 0)) {
      next.frequencyCap = 'Frequency cap must be a non‑negative number';
    }
    if (floorPriceUsd !== '' && (typeof floorPriceUsd !== 'number' || floorPriceUsd < 0)) {
      next.floorPriceUsd = 'Floor price must be a non‑negative amount';
    }
    setFieldErrors(next);
    return Object.keys(next).length === 0;
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4" role="dialog" aria-modal="true" aria-labelledby="placement-modal-title" onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}>
      <div className="w-full max-w-3xl rounded-lg border bg-white shadow-xl" tabIndex={-1}>
        <div className="flex items-start justify-between border-b p-6">
          <h2 id="placement-modal-title" className="text-gray-900 font-bold uppercase text-lg">{title}</h2>
          <button onClick={onClose} aria-label="Close dialog" className="text-gray-500 hover:text-gray-900">×</button>
        </div>

        {/* Create flow stays compact */}
        {state.type === 'create' ? (
          <div className="p-6 space-y-4">
            <p className="text-sm text-gray-700">Quickly create a new placement. You can refine details later.</p>
            <div className="flex flex-wrap gap-3">
              {(['banner','interstitial','rewarded','native'] as Placement['format'][]).map((fmt) => (
                <button key={fmt} className="btn-secondary text-sm" onClick={async () => { await onCreate({ name: `${fmt} placement`, app: 'Default App', format: fmt }); }}>
                  Create {fmt}
                </button>
              ))}
            </div>
            <div className="flex justify-end gap-3 pt-4 border-t">
              <button onClick={onClose} className="btn-outline px-6 py-3 text-sm">Close</button>
            </div>
          </div>
        ) : (
          <div className="p-6">
            {/* Tabs */}
            <div className="border-b mb-4">
              <nav className="flex gap-2" aria-label="Placement sections">
                <TabButton label="Overview" active={activeTab==='overview'} onClick={() => setActiveTab('overview')} />
                <TabButton label="Targeting" active={activeTab==='targeting'} onClick={() => setActiveTab('targeting')} />
                <TabButton label="Ad Units" active={activeTab==='adunits'} onClick={() => setActiveTab('adunits')} />
              </nav>
            </div>

            {activeTab === 'overview' && (
              <div className="space-y-4">
                <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                  <Metric label="Format" value={state.placement.format.toUpperCase()} />
                  <Metric label="eCPM" value={`$${state.placement.ecpm.toFixed(2)}`} />
                  <Metric label="Fill Rate" value={`${state.placement.fillRate.toFixed(1)}%`} />
                  <Metric label="CTR" value={`${state.placement.ctr.toFixed(2)}%`} />
                </div>
                <div className="flex items-center justify-between">
                  <div className="text-sm text-gray-600">Status: <span className="font-semibold text-gray-900">{state.placement.status}</span></div>
                  <div className="flex items-center gap-2">
                    <a href={`/dashboard/placements/${state.placement.id}`} className="btn-primary px-4 py-2 text-sm">View full report</a>
                    <button onClick={() => onArchive(state.placement.id)} className="px-4 py-2 text-sm rounded text-white bg-[rgb(var(--danger))] hover:opacity-90">Archive</button>
                  </div>
                </div>
              </div>
            )}

            {activeTab === 'targeting' && (
              <div className="space-y-4">
                <p className="text-sm text-gray-600">Basic targeting controls. This is a simplified UI; full editor will arrive with v2.</p>
                <div className="grid md:grid-cols-2 gap-4">
                  <Field label="Geos (ISO-CC)">
                    <input
                      className="input-v2 w-full"
                      placeholder="e.g., US, CA, GB"
                      value={geos}
                      onChange={(e) => setGeos(e.target.value)}
                    />
                    {fieldErrors.geos && (
                      <p className="mt-1 text-xs text-danger" role="alert">{fieldErrors.geos}</p>
                    )}
                  </Field>
                  <Field label="Platforms">
                    <select
                      className="input-v2 w-full"
                      value={platforms}
                      onChange={(e) => setPlatforms(e.target.value)}
                    >
                      <option value="iOS & Android">iOS & Android</option>
                      <option value="iOS only">iOS only</option>
                      <option value="Android only">Android only</option>
                    </select>
                  </Field>
                  <Field label="Frequency cap (per user)">
                    <input
                      type="number"
                      min={0}
                      className="input-v2 w-full"
                      placeholder="e.g., 3 per day"
                      value={frequencyCap}
                      onChange={(e) => setFrequencyCap(e.target.value === '' ? '' : Number(e.target.value))}
                    />
                    {fieldErrors.frequencyCap && (
                      <p className="mt-1 text-xs text-danger" role="alert">{fieldErrors.frequencyCap}</p>
                    )}
                  </Field>
                  <Field label="Floor price (USD)">
                    <input
                      type="number"
                      min={0}
                      step="0.01"
                      className="input-v2 w-full"
                      placeholder="e.g., 0.50"
                      value={floorPriceUsd}
                      onChange={(e) => setFloorPriceUsd(e.target.value === '' ? '' : Number(e.target.value))}
                    />
                    {fieldErrors.floorPriceUsd && (
                      <p className="mt-1 text-xs text-danger" role="alert">{fieldErrors.floorPriceUsd}</p>
                    )}
                  </Field>
                </div>
                <div className="flex justify-end pt-2">
                  <button
                    onClick={() => {
                      if (!validateTargeting()) return;
                      const currencyCode = process.env.NEXT_PUBLIC_DEFAULT_CURRENCY || 'USD';
                      const payload: any = {
                        // Proposed canonical config payload; backend may ignore until implemented
                        config: {
                          targeting: {
                            geos: toIso2Array(geos),
                            platforms: mapPlatforms(platforms),
                          },
                          delivery: {
                            ...(frequencyCap !== '' ? { frequencyCap: { count: Number(frequencyCap), per: 'day' as const } } : {}),
                          },
                          pricing: {
                            ...(floorPriceUsd !== '' ? { floorPriceCents: Math.round(Number(floorPriceUsd) * 100), currency: currencyCode } : {}),
                          },
                        },
                      };
                      onSaveConfig(state.placement.id, payload);
                    }}
                    className="btn-primary px-6 py-2 text-sm disabled:opacity-50"
                    disabled={Boolean(fieldErrors.geos || fieldErrors.frequencyCap || fieldErrors.floorPriceUsd)}
                  >
                    Save
                  </button>
                </div>
              </div>
            )}

            {activeTab === 'adunits' && (
              <div className="space-y-4">
                <p className="text-sm text-gray-600">Manage ad unit identifiers for SDK integration.</p>
                <div className="grid md:grid-cols-2 gap-4">
                  <Field label="iOS unit ID">
                    <input
                      className="input-v2 w-full font-mono"
                      placeholder="com.app.ios.banner.home"
                      value={unitIdIos}
                      onChange={(e) => setUnitIdIos(e.target.value)}
                    />
                  </Field>
                  <Field label="Android unit ID">
                    <input
                      className="input-v2 w-full font-mono"
                      placeholder="com.app.android.banner.home"
                      value={unitIdAndroid}
                      onChange={(e) => setUnitIdAndroid(e.target.value)}
                    />
                  </Field>
                </div>
                <div className="flex justify-end pt-2">
                  <button
                    onClick={() => {
                      const payload: any = {
                        config: {
                          sdk: {
                            ...(unitIdIos ? { unitIdIos } : {}),
                            ...(unitIdAndroid ? { unitIdAndroid } : {}),
                          },
                        },
                      };
                      onSaveConfig(state.placement.id, payload);
                    }}
                    className="btn-primary px-6 py-2 text-sm"
                  >
                    Save
                  </button>
                </div>
              </div>
            )}

            <div className="flex justify-end gap-3 pt-6 border-t mt-6">
              <button onClick={onClose} className="btn-outline px-6 py-3 text-sm">Close</button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function TabButton({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={`px-4 py-2 text-sm font-semibold border-b-4 ${active ? 'border-brand-500 text-brand-700' : 'border-transparent text-gray-600 hover:text-brand-700'}`}
    >
      {label}
    </button>
  );
}

function Metric({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="p-3 rounded border">
      <p className="text-xs uppercase text-gray-500 tracking-wide">{label}</p>
      <p className="text-lg font-bold text-gray-900">{value}</p>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="block text-xs font-semibold text-gray-700 mb-1">{label}</span>
      {children}
    </label>
  );
}
