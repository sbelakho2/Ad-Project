'use client';

import {
  BeakerIcon,
  ChartBarIcon,
  Cog6ToothIcon,
  CurrencyDollarIcon,
  DevicePhoneMobileIcon,
  GlobeAltIcon,
  HomeIcon,
  MapPinIcon,
  ShieldCheckIcon,
} from '@heroicons/react/24/outline';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '@/lib/api';

type DashboardSidebarProps = {
  mobileOpen?: boolean;
  onClose?: () => void;
};

type FeatureFlags = Partial<{
  revenue: boolean;
  analytics: boolean;
  networks: boolean;
  abTests: boolean;
  fraud: boolean;
  apps: boolean;
  placements: boolean;
  observability: boolean;
  reconciliation: boolean;
  transparency: boolean;
  settings: boolean;
}>;

type NavItem = { name: string; href: string; icon: any; flag?: keyof FeatureFlags };

const NAV_BLUEPRINT: NavItem[] = [
  { name: 'Dashboard', href: '/dashboard', icon: HomeIcon },
  { name: 'Revenue', href: '/dashboard/revenue', icon: CurrencyDollarIcon, flag: 'revenue' },
  { name: 'Analytics', href: '/dashboard/analytics', icon: ChartBarIcon, flag: 'analytics' },
  { name: 'Ad Networks', href: '/dashboard/networks', icon: GlobeAltIcon, flag: 'networks' },
  { name: 'A/B Tests', href: '/dashboard/ab-tests', icon: BeakerIcon, flag: 'abTests' },
  { name: 'Fraud Detection', href: '/dashboard/fraud', icon: ShieldCheckIcon, flag: 'fraud' },
  { name: 'Apps', href: '/dashboard/apps', icon: DevicePhoneMobileIcon, flag: 'apps' },
  { name: 'Placements', href: '/dashboard/placements', icon: MapPinIcon, flag: 'placements' },
  // Transparency
  { name: 'Receipts', href: '/dashboard/transparency/receipts', icon: ShieldCheckIcon, flag: 'transparency' },
  { name: 'Config Rollouts', href: '/dashboard/transparency/config-rollouts', icon: Cog6ToothIcon, flag: 'transparency' },
  // Reconciliation
  { name: 'Reconciliation', href: '/dashboard/reconciliation', icon: Cog6ToothIcon, flag: 'reconciliation' },
  // Observability
  { name: 'Observability Overview', href: '/dashboard/observability/overview', icon: ChartBarIcon, flag: 'observability' },
  { name: 'Adapter Metrics', href: '/dashboard/observability/metrics', icon: ChartBarIcon, flag: 'observability' },
  { name: 'Mediation Debugger', href: '/dashboard/observability/debugger', icon: BeakerIcon, flag: 'observability' },
  { name: 'Settings', href: '/dashboard/settings', icon: Cog6ToothIcon, flag: 'settings' },
];

export default function DashboardSidebar({ mobileOpen = false, onClose }: DashboardSidebarProps) {
  const pathname = usePathname();
  const [features, setFeatures] = useState<FeatureFlags | null>(null);
  const [loadingFlags, setLoadingFlags] = useState(true);

  // Focus trap refs for mobile drawer
  const firstFocusRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);

  // Fetch feature flags (best-effort) to gate nav visibility
  useEffect(() => {
    let alive = true;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
    (async () => {
      // Try a conventional endpoint; tolerate absence with defaults
      const res = await api.get<FeatureFlags>('/meta/features', { signal: controller?.signal });
      if (!alive) return;
      if (res.success && res.data) {
        setFeatures(res.data);
      } else {
        // Default-safe: show common pages except experimental AB tests
        setFeatures({ revenue: true, analytics: true, networks: true, fraud: true, apps: true, placements: true, transparency: true, reconciliation: true, observability: true, settings: true, abTests: false });
      }
      setLoadingFlags(false);
    })().catch(() => {
      if (!alive) return;
      setFeatures({ revenue: true, analytics: true, networks: true, fraud: true, apps: true, placements: true, transparency: true, reconciliation: true, observability: true, settings: true, abTests: false });
      setLoadingFlags(false);
    });
    return () => {
      alive = false; controller?.abort();
    };
  }, []);

  // Derive navigation based on flags
  const navigation = useMemo(() => {
    if (loadingFlags || !features) return NAV_BLUEPRINT.filter(i => !i.flag); // minimal until flags load
    return NAV_BLUEPRINT.filter((i) => !i.flag || !!(features as any)[i.flag]);
  }, [features, loadingFlags]);

  // Manage body scroll + focus trap when mobile drawer is open
  useEffect(() => {
    if (mobileOpen) {
      const prev = document.body.style.overflow;
      document.body.style.overflow = 'hidden';
      // Focus first actionable element shortly after mount
      const id = window.setTimeout(() => firstFocusRef.current?.focus(), 0);
      return () => {
        window.clearTimeout(id);
        document.body.style.overflow = prev;
      };
    }
  }, [mobileOpen]);

  useEffect(() => {
    if (!mobileOpen) return;
    function onKeyDown(e: KeyboardEvent) {
      const p = panelRef.current;
      if (!p) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose?.();
        return;
      }
      if (e.key === 'Tab') {
        const focusables = p.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), textarea, input, select, [tabindex]:not([tabindex="-1"])'
        );
        if (focusables.length === 0) return;
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          (first as HTMLElement).focus();
        } else if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          (last as HTMLElement).focus();
        }
      }
    }
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [mobileOpen, onClose]);

  const handleNavigate = () => onClose?.();

  return (
    <>
      {/* Desktop sidebar */}
      <div className="hidden lg:fixed lg:inset-y-0 lg:flex lg:w-[280px] lg:flex-col">
        <div className="flex flex-grow flex-col overflow-y-auto bg-white pt-5 pb-4 border-r border-gray-200">
          <div className="flex flex-shrink-0 items-center px-4">
            <h1 className="text-xl font-semibold tracking-tight text-gray-900">ApexMediation</h1>
          </div>
          <nav className="mt-4 flex flex-1 flex-col overflow-y-auto" aria-label="Sidebar">
            <div className="space-y-1 px-3">
              {navigation.map((item) => {
                const isActive = pathname === item.href;
                return (
                  <Link
                    key={item.name}
                    href={item.href}
                    className={`group flex items-center gap-3 rounded-[999px] px-3 py-2 text-sm font-medium leading-6 transition-colors ${
                      isActive
                        ? 'bg-brand-50 text-brand-700 border border-brand-500'
                        : 'text-gray-700 hover:bg-brand-50 hover:text-brand-700'
                    }`}
                  >
                    <item.icon className={`h-5 w-5 flex-shrink-0 ${isActive ? 'text-brand-600' : 'text-gray-400 group-hover:text-brand-600'}`} aria-hidden="true" />
                    {item.name}
                  </Link>
                );
              })}
            </div>
          </nav>
        </div>
      </div>

      {/* Mobile sidebar drawer */}
      {mobileOpen && (
        <div className="lg:hidden" role="dialog" aria-modal="true" aria-labelledby="mobile-sidebar-title">
          <div className="fixed inset-0 z-40 bg-black/50" onClick={onClose} aria-hidden="true" />
          <div
            ref={panelRef}
            className="fixed inset-y-0 right-0 z-50 flex w-72 flex-col bg-white pt-5 pb-6 shadow-2xl"
          >
            <div className="flex items-center justify-between px-4">
              <h2 id="mobile-sidebar-title" className="text-lg font-semibold tracking-tight text-gray-900">
                Navigate
              </h2>
              <button
                type="button"
                onClick={onClose}
                ref={firstFocusRef}
                className="rounded-full border px-3 py-1 text-gray-700 transition hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2"
              >
                Close
              </button>
            </div>
            <nav className="mt-6 flex-1 overflow-y-auto" aria-label="Mobile sidebar">
              <div className="space-y-1 px-4">
                {navigation.map((item) => {
                  const isActive = pathname === item.href;
                  return (
                    <Link
                      key={item.name}
                      href={item.href}
                      onClick={handleNavigate}
                      className={`flex items-center gap-3 rounded-xl px-3 py-3 text-sm font-medium transition ${
                        isActive ? 'bg-brand-50 text-brand-700 border border-brand-500' : 'text-gray-700 hover:bg-gray-50'
                      }`}
                    >
                      <item.icon className={`h-5 w-5 ${isActive ? 'text-brand-600' : 'text-gray-400'}`} aria-hidden="true" />
                      {item.name}
                    </Link>
                  );
                })}
              </div>
            </nav>
          </div>
        </div>
      )}
    </>
  );
}
