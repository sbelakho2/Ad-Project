'use client';

import { useEffect, useState } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { usePlanAccess } from '@/lib/usePlanAccess';

interface Connection {
  id: string;
  networkType: string;
  networkName: string;
  status: string;
  lastSyncAt: string | null;
}

interface ReconOverview {
  coveragePercent: number;
  variancePercent: number;
  totals: {
    impressions: number;
    paid: number;
    expected: number;
  };
  byNetwork: Array<{
    network: string;
    impressions: number;
    paid: number;
    expected: number;
    variance?: number;
  }>;
}

interface Alert {
  id: string;
  alertType: string;
  severity: string;
  message: string;
  createdAt: string;
}

export default function ReconDashboardPage() {
  const router = useRouter();
  const { allowed, isLoading: planLoading, plan } = usePlanAccess(['growth', 'scale', 'enterprise']);
  const [connections, setConnections] = useState<Connection[]>([]);
  const [overview, setOverview] = useState<ReconOverview | null>(null);
  const [alerts, setAlerts] = useState<Alert[]>([]);
  const [loading, setLoading] = useState(true);
  const [reconciling, setReconciling] = useState(false);

  useEffect(() => {
    if (planLoading || !allowed) {
      setLoading(false);
      return;
    }
    fetchData();
  }, [planLoading, allowed]);

  async function fetchData() {
    setLoading(true);
    try {
      const [connRes, overviewRes, alertsRes] = await Promise.all([
        fetch('/api/v1/vra/connections', { credentials: 'include' }),
        fetch('/api/v1/recon/overview', { credentials: 'include' }),
        fetch('/api/v1/vra/alerts?status=open&limit=5', { credentials: 'include' }),
      ]);

      if (connRes.ok) {
        const data = await connRes.json();
        setConnections(data.connections || []);
      }

      if (overviewRes.ok) {
        const data = await overviewRes.json();
        setOverview(data.data || null);
      }

      if (alertsRes.ok) {
        const data = await alertsRes.json();
        setAlerts(data.alerts || []);
      }
    } catch (err) {
      console.error('Failed to fetch data:', err);
    } finally {
      setLoading(false);
    }
  }

  async function runReconciliation() {
    setReconciling(true);
    try {
      const res = await fetch('/api/v1/vra/reconcile', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({}),
      });
      
      if (res.ok) {
        await fetchData();
      }
    } catch (err) {
      console.error('Reconciliation failed:', err);
    } finally {
      setReconciling(false);
    }
  }

  const activeConnections = connections.filter(c => c.status === 'active');
  const hasConnections = connections.length > 0;
  
  const varianceUsd = overview ? overview.totals.expected - overview.totals.paid : 0;
  const varianceIsNegative = varianceUsd > 0; // Underpayment

  if (planLoading) {
    return (
      <div className="p-6">
        <p className="text-sm text-gray-500">Checking your plan…</p>
      </div>
    );
  }

  if (!allowed) {
    return (
      <div className="p-6">
        <h1 className="text-xl font-semibold mb-4">Revenue Reconciliation</h1>
        <div className="rounded-md border border-amber-200 bg-amber-50 p-4" role="alert">
          <p className="text-sm font-semibold text-amber-900">Upgrade to access VRA reconciliation</p>
          <p className="text-sm text-amber-800 mt-1">
            Starter is limited to core dashboards. VRA reconciliation is available on Growth and above. Current plan: {plan || 'unknown'}.
          </p>
          <div className="mt-3 flex gap-2 flex-wrap">
            <a href="/settings/billing/settings" className="px-3 py-2 text-sm font-semibold text-primary-700 border border-primary-200 rounded-lg hover:bg-primary-50">
              Open Billing
            </a>
          </div>
        </div>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="p-6">
        <p className="text-sm text-gray-500">Loading…</p>
      </div>
    );
  }

  // Show setup wizard if no connections
  if (!hasConnections) {
    return (
      <div className="p-6 max-w-4xl mx-auto">
        <div className="text-center mb-8">
          <h1 className="text-2xl font-bold mb-2">Revenue Reconciliation</h1>
          <p className="text-gray-600">
            Automatically track and reconcile your ad revenue across all networks.
          </p>
        </div>

        <div className="rounded-md border bg-white p-6 mb-6">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
            </svg>
            Get Started
          </h2>
          <p className="text-sm text-gray-600 mb-6">
            Connect your ad networks to enable automated reconciliation.
          </p>

          <div className="space-y-4">
            <div className="flex items-start gap-4 p-4 rounded-lg bg-gray-50">
              <div className="rounded-full bg-blue-100 px-3 py-1">
                <span className="text-sm font-bold text-blue-700">1</span>
              </div>
              <div>
                <h3 className="font-medium">Connect your ad networks</h3>
                <p className="text-sm text-gray-600">
                  We support AdMob, Unity, AppLovin, ironSource, Meta, and Vungle.
                </p>
              </div>
            </div>

            <div className="flex items-start gap-4 p-4 rounded-lg bg-gray-50">
              <div className="rounded-full bg-blue-100 px-3 py-1">
                <span className="text-sm font-bold text-blue-700">2</span>
              </div>
              <div>
                <h3 className="font-medium">We sync your revenue data</h3>
                <p className="text-sm text-gray-600">
                  Revenue statements are automatically fetched every 6 hours from each network.
                </p>
              </div>
            </div>

            <div className="flex items-start gap-4 p-4 rounded-lg bg-gray-50">
              <div className="rounded-full bg-blue-100 px-3 py-1">
                <span className="text-sm font-bold text-blue-700">3</span>
              </div>
              <div>
                <h3 className="font-medium">View variances and alerts</h3>
                <p className="text-sm text-gray-600">
                  We compare SDK events against network statements and alert you to discrepancies.
                </p>
              </div>
            </div>

            <button
              onClick={() => router.push('/settings/networks')}
              className="w-full mt-4 px-4 py-3 bg-blue-600 text-white font-semibold rounded-lg hover:bg-blue-700 flex items-center justify-center gap-2"
            >
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
              </svg>
              Connect Your First Network
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">Revenue Reconciliation</h1>
          <p className="text-sm text-gray-600">
            Track variances between SDK events and network statements.
          </p>
        </div>
        <button
          onClick={runReconciliation}
          disabled={reconciling}
          className="px-4 py-2 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700 disabled:opacity-50 flex items-center gap-2"
        >
          {reconciling ? (
            <svg className="w-4 h-4 animate-spin" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          ) : (
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          )}
          Run Reconciliation
        </button>
      </div>

      {/* Key Metrics */}
      <section className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="rounded-md border p-4 bg-white">
          <div className="text-xs text-gray-500 uppercase">Coverage</div>
          <div className="text-2xl font-semibold mt-1">
            {overview?.coveragePercent?.toFixed(0) || 0}%
          </div>
          <div className="mt-2 h-2 bg-gray-200 rounded-full overflow-hidden">
            <div 
              className="h-full bg-blue-600 rounded-full transition-all"
              style={{ width: `${overview?.coveragePercent || 0}%` }}
            />
          </div>
          <p className="text-xs text-gray-500 mt-2">Networks with statements</p>
        </div>

        <div className="rounded-md border p-4 bg-white">
          <div className="text-xs text-gray-500 uppercase">Expected Revenue</div>
          <div className="text-2xl font-semibold mt-1">
            ${(overview?.totals?.expected || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })}
          </div>
          <p className="text-xs text-gray-500 mt-2">From SDK events (last 7 days)</p>
        </div>

        <div className="rounded-md border p-4 bg-white">
          <div className="text-xs text-gray-500 uppercase">Paid by Networks</div>
          <div className="text-2xl font-semibold mt-1">
            ${(overview?.totals?.paid || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })}
          </div>
          <p className="text-xs text-gray-500 mt-2">From network statements</p>
        </div>

        <div className={`rounded-md border p-4 bg-white ${varianceIsNegative ? 'border-red-300' : ''}`}>
          <div className="text-xs text-gray-500 uppercase">Variance</div>
          <div className={`text-2xl font-semibold mt-1 flex items-center gap-2 ${varianceIsNegative ? 'text-red-600' : 'text-green-600'}`}>
            {varianceIsNegative ? (
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 17h8m0 0V9m0 8l-8-8-4 4-6-6" />
              </svg>
            ) : (
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6" />
              </svg>
            )}
            ${Math.abs(varianceUsd).toLocaleString(undefined, { maximumFractionDigits: 0 })}
          </div>
          <p className="text-xs text-gray-500 mt-2">
            {varianceIsNegative ? 'Potential underpayment' : 'Networks paid more than expected'}
          </p>
        </div>
      </section>

      <div className="grid gap-6 md:grid-cols-2">
        {/* Network Status */}
        <div className="rounded-md border p-4 bg-white">
          <h2 className="text-base font-semibold flex items-center gap-2 mb-1">
            <svg className="w-5 h-5 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
            </svg>
            Connected Networks
          </h2>
          <p className="text-xs text-gray-500 mb-4">
            {activeConnections.length} of {connections.length} active
          </p>

          <div className="space-y-3">
            {connections.slice(0, 5).map((conn) => (
              <div key={conn.id} className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className={`inline-flex items-center px-2 py-1 text-xs font-medium rounded ${
                    conn.status === 'active' 
                      ? 'bg-green-100 text-green-700' 
                      : 'bg-gray-100 text-gray-700'
                  }`}>
                    {conn.status === 'active' ? '✓' : '!'} {conn.status}
                  </span>
                  <span className="font-medium text-sm">{conn.networkName}</span>
                </div>
                <span className="text-xs text-gray-500">
                  {conn.lastSyncAt 
                    ? `Synced ${new Date(conn.lastSyncAt).toLocaleDateString()}`
                    : 'Not synced'}
                </span>
              </div>
            ))}
          </div>
          <Link 
            href="/settings/networks"
            className="mt-4 w-full inline-flex items-center justify-center px-3 py-2 text-sm border rounded-lg hover:bg-gray-50"
          >
            Manage Connections
            <svg className="w-4 h-4 ml-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          </Link>
        </div>

        {/* Per-Network Variance */}
        <div className="rounded-md border p-4 bg-white">
          <h2 className="text-base font-semibold flex items-center gap-2 mb-1">
            <svg className="w-5 h-5 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
            </svg>
            Variance by Network
          </h2>
          <p className="text-xs text-gray-500 mb-4">Revenue comparison per network</p>

          <div className="space-y-3">
            {(overview?.byNetwork || []).slice(0, 5).map((net) => {
              const variance = net.expected - net.paid;
              const variancePct = net.expected > 0 ? (variance / net.expected) * 100 : 0;
              const isUnderpay = variance > 0;
              
              return (
                <div key={net.network} className="flex items-center justify-between">
                  <div>
                    <span className="font-medium text-sm">{net.network}</span>
                    <span className="text-xs text-gray-500 ml-2">
                      ${net.paid.toLocaleString(undefined, { maximumFractionDigits: 0 })} paid
                    </span>
                  </div>
                  <span className={`inline-flex items-center px-2 py-1 text-xs font-medium rounded ${
                    Math.abs(variancePct) > 5 
                      ? 'bg-red-100 text-red-700' 
                      : 'bg-gray-100 text-gray-700'
                  }`}>
                    {isUnderpay ? '-' : '+'}${Math.abs(variance).toFixed(2)}
                    ({variancePct.toFixed(1)}%)
                  </span>
                </div>
              );
            })}
            {(!overview?.byNetwork || overview.byNetwork.length === 0) && (
              <p className="text-sm text-gray-500 text-center py-4">
                No network data available yet. Sync your networks to see variance data.
              </p>
            )}
          </div>
          <Link 
            href="/recon/deltas"
            className="mt-4 w-full inline-flex items-center justify-center px-3 py-2 text-sm border rounded-lg hover:bg-gray-50"
          >
            View All Deltas
            <svg className="w-4 h-4 ml-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          </Link>
        </div>
      </div>

      {/* Alerts Section */}
      {alerts.length > 0 && (
        <div className="rounded-md border p-4 bg-white">
          <h2 className="text-base font-semibold flex items-center gap-2 mb-4">
            <svg className="w-5 h-5 text-yellow-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
            Active Alerts
          </h2>
          <div className="space-y-3">
            {alerts.map((alert) => (
              <div
                key={alert.id}
                className={`flex items-start gap-3 p-3 rounded-lg ${
                  alert.severity === 'critical' ? 'bg-red-50' : 'bg-yellow-50'
                }`}
              >
                <svg className={`w-5 h-5 mt-0.5 ${
                  alert.severity === 'critical' ? 'text-red-500' : 'text-yellow-500'
                }`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                <div>
                  <p className="font-medium text-sm">{alert.message}</p>
                  <p className="text-xs text-gray-500">
                    {new Date(alert.createdAt).toLocaleString()}
                  </p>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
