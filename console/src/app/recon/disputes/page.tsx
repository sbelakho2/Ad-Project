'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import {
  AlertCircle,
  ArrowRight,
  CheckCircle2,
  Clock,
  DollarSign,
  FileText,
  Filter,
  Loader2,
  Plus,
  RefreshCw,
  Send,
  XCircle,
} from 'lucide-react';
import type { DisputeRecord, DisputeStats, DisputeStatus, DisputeListQuery } from '@/lib/vra';
import { listDisputes, getDisputeStats, submitDispute, cancelDispute } from '@/lib/vra';
import { usePlanAccess } from '@/lib/usePlanAccess';

const STATUS_CONFIG: Record<DisputeStatus, { label: string; color: string; icon: typeof Clock }> = {
  draft: { label: 'Draft', color: 'bg-gray-100 text-gray-700 border-gray-200', icon: FileText },
  submitted: { label: 'Submitted', color: 'bg-blue-100 text-blue-700 border-blue-200', icon: Send },
  in_progress: { label: 'In Progress', color: 'bg-yellow-100 text-yellow-700 border-yellow-200', icon: Clock },
  resolved: { label: 'Resolved', color: 'bg-green-100 text-green-700 border-green-200', icon: CheckCircle2 },
  rejected: { label: 'Rejected', color: 'bg-red-100 text-red-700 border-red-200', icon: XCircle },
  cancelled: { label: 'Cancelled', color: 'bg-gray-100 text-gray-500 border-gray-200', icon: XCircle },
};

function StatusBadge({ status }: { status: DisputeStatus }) {
  const config = STATUS_CONFIG[status];
  const Icon = config.icon;
  return (
    <span className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium ${config.color}`}>
      <Icon className="h-3.5 w-3.5" />
      {config.label}
    </span>
  );
}

function StatCard({
  label,
  value,
  subValue,
  icon: Icon,
  trend,
}: {
  label: string;
  value: string | number;
  subValue?: string;
  icon: typeof DollarSign;
  trend?: 'up' | 'down' | 'neutral';
}) {
  return (
    <div className="rounded-xl border border-gray-100 bg-white p-5 shadow-sm">
      <div className="flex items-center justify-between">
        <div className="rounded-lg bg-primary-50 p-2.5">
          <Icon className="h-5 w-5 text-primary-600" />
        </div>
        {trend && (
          <span className={`text-xs font-medium ${
            trend === 'up' ? 'text-green-600' : trend === 'down' ? 'text-red-600' : 'text-gray-500'
          }`}>
            {trend === 'up' ? '↑' : trend === 'down' ? '↓' : '—'}
          </span>
        )}
      </div>
      <div className="mt-4">
        <p className="text-2xl font-semibold text-gray-900">{value}</p>
        <p className="mt-1 text-sm text-gray-500">{label}</p>
        {subValue && <p className="mt-0.5 text-xs text-gray-400">{subValue}</p>}
      </div>
    </div>
  );
}

export default function DisputeCenterPage() {
  const router = useRouter();
  const { allowed, isLoading: planLoading, plan } = usePlanAccess(['growth', 'scale', 'enterprise']);
  
  const [disputes, setDisputes] = useState<DisputeRecord[]>([]);
  const [stats, setStats] = useState<DisputeStats | null>(null);
  const [pagination, setPagination] = useState({ page: 1, pageSize: 20, total: 0, totalPages: 1 });
  const [loading, setLoading] = useState(true);
  const [statsLoading, setStatsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  
  // Filters
  const [statusFilter, setStatusFilter] = useState<DisputeStatus | ''>('');
  const [networkFilter, setNetworkFilter] = useState<string>('');

  const loadDisputes = useCallback(async () => {
    if (!allowed) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const query: DisputeListQuery = {
        page: pagination.page,
        page_size: pagination.pageSize,
      };
      if (statusFilter) query.status = statusFilter;
      if (networkFilter) query.network = networkFilter;
      
      const result = await listDisputes(query);
      setDisputes(result.items);
      setPagination(result.pagination);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load disputes');
    } finally {
      setLoading(false);
    }
  }, [allowed, pagination.page, pagination.pageSize, statusFilter, networkFilter]);

  const loadStats = useCallback(async () => {
    if (!allowed) {
      setStatsLoading(false);
      return;
    }
    setStatsLoading(true);
    try {
      const result = await getDisputeStats();
      setStats(result);
    } catch {
      // Stats are optional, don't block on error
    } finally {
      setStatsLoading(false);
    }
  }, [allowed]);

  useEffect(() => {
    loadDisputes();
    loadStats();
  }, [loadDisputes, loadStats]);

  const handleSubmit = useCallback(async (id: string) => {
    if (actionLoading) return;
    setActionLoading(id);
    try {
      await submitDispute(id);
      await loadDisputes();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to submit dispute');
    } finally {
      setActionLoading(null);
    }
  }, [actionLoading, loadDisputes]);

  const handleCancel = useCallback(async (id: string) => {
    if (actionLoading) return;
    if (!confirm('Are you sure you want to cancel this dispute?')) return;
    setActionLoading(id);
    try {
      await cancelDispute(id);
      await loadDisputes();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to cancel dispute');
    } finally {
      setActionLoading(null);
    }
  }, [actionLoading, loadDisputes]);

  const uniqueNetworks = useMemo(() => {
    const networks = new Set(disputes.map(d => d.network));
    return Array.from(networks).sort();
  }, [disputes]);

  if (planLoading) {
    return (
      <div className="p-6">
        <div className="flex items-center gap-2 text-gray-500">
          <Loader2 className="h-4 w-4 animate-spin" />
          <span>Checking your plan…</span>
        </div>
      </div>
    );
  }

  if (!allowed) {
    return (
      <div className="p-6">
        <div className="rounded-xl border border-amber-200 bg-amber-50 p-6">
          <div className="flex items-start gap-3">
            <AlertCircle className="h-5 w-5 text-amber-600 flex-shrink-0 mt-0.5" />
            <div>
              <h3 className="font-semibold text-amber-900">Upgrade Required</h3>
              <p className="mt-1 text-sm text-amber-800">
                The Dispute Center is available on Growth plans and above. Current plan: {plan || 'Starter'}.
              </p>
              <div className="mt-4 flex gap-3">
                <Link
                  href="/settings/billing"
                  className="inline-flex items-center gap-1.5 rounded-lg bg-amber-600 px-4 py-2 text-sm font-medium text-white hover:bg-amber-700"
                >
                  Upgrade Plan
                  <ArrowRight className="h-4 w-4" />
                </Link>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold text-gray-900">Dispute Center</h1>
          <p className="mt-1 text-sm text-gray-500">
            Track and manage revenue discrepancy claims with ad networks
          </p>
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={() => { loadDisputes(); loadStats(); }}
            disabled={loading}
            className="inline-flex items-center gap-2 rounded-lg border border-gray-200 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
          >
            <RefreshCw className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
            Refresh
          </button>
          <Link
            href="/recon/deltas"
            className="inline-flex items-center gap-2 rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700"
          >
            <Plus className="h-4 w-4" />
            Create from Delta
          </Link>
        </div>
      </div>

      {/* Stats Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <StatCard
          label="Total Disputes"
          value={statsLoading ? '—' : (stats?.total ?? 0)}
          icon={FileText}
        />
        <StatCard
          label="Amount Disputed"
          value={statsLoading ? '—' : `$${(stats?.totalAmountDisputed ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}`}
          icon={DollarSign}
        />
        <StatCard
          label="Amount Recovered"
          value={statsLoading ? '—' : `$${(stats?.totalAmountResolved ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}`}
          subValue={stats && stats.totalAmountDisputed > 0
            ? `${((stats.totalAmountResolved / stats.totalAmountDisputed) * 100).toFixed(1)}% recovery rate`
            : undefined}
          icon={CheckCircle2}
          trend={stats && stats.totalAmountResolved > 0 ? 'up' : 'neutral'}
        />
        <StatCard
          label="Avg Resolution Time"
          value={statsLoading ? '—' : (stats?.avgResolutionDays != null ? `${stats.avgResolutionDays.toFixed(1)} days` : 'N/A')}
          icon={Clock}
        />
      </div>

      {/* Status Summary Pills */}
      {stats && (
        <div className="flex flex-wrap gap-2">
          {(Object.entries(stats.byStatus) as [DisputeStatus, number][])
            .filter(([, count]) => count > 0)
            .map(([status, count]) => (
              <button
                key={status}
                onClick={() => setStatusFilter(status === statusFilter ? '' : status)}
                className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-medium transition-colors ${
                  status === statusFilter ? 'ring-2 ring-primary-500 ring-offset-1' : ''
                } ${STATUS_CONFIG[status].color}`}
              >
                {STATUS_CONFIG[status].label}: {count}
              </button>
            ))}
        </div>
      )}

      {/* Filters */}
      <div className="flex flex-col sm:flex-row gap-4 items-start sm:items-center">
        <div className="flex items-center gap-2 text-sm text-gray-500">
          <Filter className="h-4 w-4" />
          <span>Filters:</span>
        </div>
        <select
          value={statusFilter}
          onChange={(e) => { setStatusFilter(e.target.value as DisputeStatus | ''); setPagination(p => ({ ...p, page: 1 })); }}
          className="rounded-lg border border-gray-200 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary-500"
        >
          <option value="">All Statuses</option>
          {Object.entries(STATUS_CONFIG).map(([status, config]) => (
            <option key={status} value={status}>{config.label}</option>
          ))}
        </select>
        {uniqueNetworks.length > 0 && (
          <select
            value={networkFilter}
            onChange={(e) => { setNetworkFilter(e.target.value); setPagination(p => ({ ...p, page: 1 })); }}
            className="rounded-lg border border-gray-200 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary-500"
          >
            <option value="">All Networks</option>
            {uniqueNetworks.map(n => (
              <option key={n} value={n}>{n}</option>
            ))}
          </select>
        )}
        {(statusFilter || networkFilter) && (
          <button
            onClick={() => { setStatusFilter(''); setNetworkFilter(''); }}
            className="text-sm text-primary-600 hover:text-primary-700"
          >
            Clear filters
          </button>
        )}
      </div>

      {/* Error Alert */}
      {error && (
        <div className="rounded-lg border border-red-200 bg-red-50 p-4 flex items-start gap-3">
          <AlertCircle className="h-5 w-5 text-red-600 flex-shrink-0" />
          <div>
            <p className="text-sm font-medium text-red-800">{error}</p>
            <button onClick={() => setError(null)} className="mt-1 text-xs text-red-600 hover:underline">
              Dismiss
            </button>
          </div>
        </div>
      )}

      {/* Disputes Table */}
      <div className="rounded-xl border border-gray-100 bg-white shadow-sm overflow-hidden">
        {loading ? (
          <div className="flex items-center justify-center p-12">
            <Loader2 className="h-6 w-6 animate-spin text-gray-400" />
          </div>
        ) : disputes.length === 0 ? (
          <div className="p-12 text-center">
            <FileText className="h-12 w-12 text-gray-300 mx-auto" />
            <p className="mt-4 text-gray-600 font-medium">No disputes found</p>
            <p className="mt-1 text-sm text-gray-500">
              Create a dispute from the Deltas page when you identify discrepancies.
            </p>
            <Link
              href="/recon/deltas"
              className="mt-4 inline-flex items-center gap-2 text-sm text-primary-600 hover:text-primary-700"
            >
              Go to Deltas
              <ArrowRight className="h-4 w-4" />
            </Link>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-100">
              <thead>
                <tr className="bg-gray-50">
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Dispute ID
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Network
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Amount
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Status
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Created
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Resolution
                  </th>
                  <th className="px-4 py-3 text-right text-xs font-semibold text-gray-600 uppercase tracking-wider">
                    Actions
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100">
                {disputes.map((dispute) => (
                  <tr key={dispute.id} className="hover:bg-gray-50 transition-colors">
                    <td className="px-4 py-4">
                      <Link
                        href={`/recon/disputes/${dispute.id}`}
                        className="text-sm font-medium text-primary-600 hover:text-primary-700"
                      >
                        {dispute.disputeId.slice(0, 8)}…
                      </Link>
                    </td>
                    <td className="px-4 py-4">
                      <span className="text-sm text-gray-900">{dispute.network}</span>
                    </td>
                    <td className="px-4 py-4">
                      <span className="text-sm font-medium text-gray-900">
                        ${dispute.amount.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                      </span>
                    </td>
                    <td className="px-4 py-4">
                      <StatusBadge status={dispute.status} />
                    </td>
                    <td className="px-4 py-4">
                      <span className="text-sm text-gray-600">
                        {new Date(dispute.createdAt).toLocaleDateString()}
                      </span>
                    </td>
                    <td className="px-4 py-4">
                      {dispute.status === 'resolved' && dispute.resolutionAmount != null ? (
                        <span className="text-sm font-medium text-green-600">
                          ${dispute.resolutionAmount.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                        </span>
                      ) : dispute.status === 'rejected' ? (
                        <span className="text-sm text-red-600">Denied</span>
                      ) : (
                        <span className="text-sm text-gray-400">—</span>
                      )}
                    </td>
                    <td className="px-4 py-4 text-right">
                      <div className="flex items-center justify-end gap-2">
                        {dispute.status === 'draft' && (
                          <>
                            <button
                              onClick={() => handleSubmit(dispute.id)}
                              disabled={actionLoading === dispute.id}
                              className="inline-flex items-center gap-1 rounded-md bg-primary-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-primary-700 disabled:opacity-50"
                            >
                              {actionLoading === dispute.id ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <Send className="h-3.5 w-3.5" />
                              )}
                              Submit
                            </button>
                            <button
                              onClick={() => handleCancel(dispute.id)}
                              disabled={actionLoading === dispute.id}
                              className="inline-flex items-center gap-1 rounded-md border border-gray-200 px-2.5 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
                            >
                              Cancel
                            </button>
                          </>
                        )}
                        {(dispute.status === 'submitted' || dispute.status === 'in_progress') && (
                          <button
                            onClick={() => handleCancel(dispute.id)}
                            disabled={actionLoading === dispute.id}
                            className="inline-flex items-center gap-1 rounded-md border border-gray-200 px-2.5 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
                          >
                            {actionLoading === dispute.id && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                            Withdraw
                          </button>
                        )}
                        <Link
                          href={`/recon/disputes/${dispute.id}`}
                          className="inline-flex items-center gap-1 rounded-md border border-gray-200 px-2.5 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50"
                        >
                          View
                          <ArrowRight className="h-3.5 w-3.5" />
                        </Link>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {/* Pagination */}
        {pagination.totalPages > 1 && (
          <div className="flex items-center justify-between border-t border-gray-100 px-4 py-3">
            <p className="text-sm text-gray-600">
              Showing {((pagination.page - 1) * pagination.pageSize) + 1} to{' '}
              {Math.min(pagination.page * pagination.pageSize, pagination.total)} of {pagination.total}
            </p>
            <div className="flex items-center gap-2">
              <button
                onClick={() => setPagination(p => ({ ...p, page: p.page - 1 }))}
                disabled={pagination.page <= 1}
                className="rounded-md border border-gray-200 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Previous
              </button>
              <span className="text-sm text-gray-600">
                Page {pagination.page} of {pagination.totalPages}
              </span>
              <button
                onClick={() => setPagination(p => ({ ...p, page: p.page + 1 }))}
                disabled={pagination.page >= pagination.totalPages}
                className="rounded-md border border-gray-200 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Next
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
