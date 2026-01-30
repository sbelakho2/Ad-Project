'use client';

import { useCallback, useEffect, useState } from 'react';
import Link from 'next/link';
import {
  AlertCircle,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronUp,
  HelpCircle,
  Loader2,
  RefreshCw,
  X,
  Zap,
} from 'lucide-react';
import type { ReviewQueueItem, ReviewQueueQuery } from '@/lib/vra';
import { getReviewQueue, approveReviewItem, rejectReviewItem } from '@/lib/vra';
import { usePlanAccess } from '@/lib/usePlanAccess';

function ConfidenceBar({ confidence }: { confidence: number }) {
  const percent = Math.round(confidence * 100);
  let color = 'bg-red-500';
  let label = 'Low';
  if (confidence >= 0.7) {
    color = 'bg-yellow-500';
    label = 'Medium-High';
  } else if (confidence >= 0.5) {
    color = 'bg-yellow-400';
    label = 'Medium';
  } else if (confidence >= 0.4) {
    color = 'bg-orange-400';
    label = 'Low-Medium';
  }

  return (
    <div className="flex items-center gap-2">
      <div className="w-24 h-2 bg-gray-100 rounded-full overflow-hidden">
        <div className={`h-full ${color} transition-all`} style={{ width: `${percent}%` }} />
      </div>
      <span className="text-xs font-medium text-gray-600">{percent}%</span>
      <span className="text-xs text-gray-400">({label})</span>
    </div>
  );
}

function ReviewItemCard({
  item,
  onApprove,
  onReject,
  loading,
  expanded,
  onToggle,
}: {
  item: ReviewQueueItem;
  onApprove: () => void;
  onReject: () => void;
  loading: boolean;
  expanded: boolean;
  onToggle: () => void;
}) {
  const variance = item.expectedAmount - item.statementAmount;
  const variancePercent = item.expectedAmount > 0 
    ? ((variance / item.expectedAmount) * 100).toFixed(1)
    : 0;

  return (
    <div className="rounded-xl border border-gray-100 bg-white shadow-sm overflow-hidden transition-all">
      {/* Header - always visible */}
      <div className="p-4 sm:p-5">
        <div className="flex flex-col sm:flex-row sm:items-center gap-4">
          {/* Main info */}
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium text-gray-900">{item.networkType || 'Unknown Network'}</span>
              <span className="text-xs text-gray-400">•</span>
              <span className="text-xs text-gray-500">{item.eventDate || 'No date'}</span>
            </div>
            <div className="mt-2">
              <ConfidenceBar confidence={item.confidence} />
            </div>
          </div>

          {/* Amounts */}
          <div className="flex items-center gap-6 text-sm">
            <div>
              <div className="text-xs text-gray-500">Statement</div>
              <div className="font-medium text-gray-900">
                ${item.statementAmount.toLocaleString(undefined, { maximumFractionDigits: 2 })}
              </div>
            </div>
            <div>
              <div className="text-xs text-gray-500">Expected</div>
              <div className="font-medium text-gray-900">
                ${item.expectedAmount.toLocaleString(undefined, { maximumFractionDigits: 2 })}
              </div>
            </div>
            <div>
              <div className="text-xs text-gray-500">Variance</div>
              <div className={`font-medium ${variance > 0 ? 'text-red-600' : variance < 0 ? 'text-green-600' : 'text-gray-600'}`}>
                {variance > 0 ? '+' : ''}${variance.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                <span className="text-xs ml-1">({variancePercent}%)</span>
              </div>
            </div>
          </div>

          {/* Actions */}
          <div className="flex items-center gap-2">
            <button
              onClick={onApprove}
              disabled={loading}
              className="inline-flex items-center gap-1.5 rounded-lg bg-green-600 px-3 py-2 text-sm font-medium text-white hover:bg-green-700 disabled:opacity-50 transition-colors"
              title="Approve this match"
            >
              {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Check className="h-4 w-4" />}
              <span className="hidden sm:inline">Approve</span>
            </button>
            <button
              onClick={onReject}
              disabled={loading}
              className="inline-flex items-center gap-1.5 rounded-lg border border-red-200 bg-white px-3 py-2 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 transition-colors"
              title="Reject this match"
            >
              {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <X className="h-4 w-4" />}
              <span className="hidden sm:inline">Reject</span>
            </button>
            <button
              onClick={onToggle}
              className="inline-flex items-center gap-1 rounded-lg border border-gray-200 bg-white px-3 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
            >
              {expanded ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </button>
          </div>
        </div>
      </div>

      {/* Expanded details */}
      {expanded && (
        <div className="border-t border-gray-100 bg-gray-50 p-4 sm:p-5 space-y-4">
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 text-sm">
            <div>
              <span className="text-xs text-gray-500 uppercase tracking-wide">Statement ID</span>
              <p className="mt-1 font-mono text-xs text-gray-700 break-all">{item.statementId}</p>
            </div>
            <div>
              <span className="text-xs text-gray-500 uppercase tracking-wide">Request ID</span>
              <p className="mt-1 font-mono text-xs text-gray-700 break-all">{item.requestId}</p>
            </div>
            <div>
              <span className="text-xs text-gray-500 uppercase tracking-wide">Keys Used</span>
              <p className="mt-1 font-mono text-xs text-gray-700">{item.keysUsed || 'N/A'}</p>
            </div>
          </div>
          {item.reasons && Object.keys(item.reasons).length > 0 && (
            <div>
              <span className="text-xs text-gray-500 uppercase tracking-wide">Match Reasons</span>
              <div className="mt-2 flex flex-wrap gap-2">
                {Object.entries(item.reasons).map(([key, value]) => (
                  <span
                    key={key}
                    className="inline-flex items-center rounded-full bg-white border border-gray-200 px-2.5 py-1 text-xs font-medium text-gray-700"
                  >
                    {key}: {String(value)}
                  </span>
                ))}
              </div>
            </div>
          )}
          <div className="text-xs text-gray-400">
            Matched at: {new Date(item.createdAt).toLocaleString()}
          </div>
        </div>
      )}
    </div>
  );
}

export default function ReviewQueuePage() {
  const { allowed, isLoading: planLoading, plan } = usePlanAccess(['growth', 'scale', 'enterprise']);
  
  const [items, setItems] = useState<ReviewQueueItem[]>([]);
  const [pagination, setPagination] = useState({ page: 1, pageSize: 20, total: 0, totalPages: 1 });
  const [filters, setFilters] = useState({ minConf: 0.4, maxConf: 0.8 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  // Filter inputs
  const [minConfInput, setMinConfInput] = useState(40);
  const [maxConfInput, setMaxConfInput] = useState(80);

  const loadQueue = useCallback(async () => {
    if (!allowed) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const query: ReviewQueueQuery = {
        page: pagination.page,
        page_size: pagination.pageSize,
        min_conf: minConfInput / 100,
        max_conf: maxConfInput / 100,
      };
      const result = await getReviewQueue(query);
      setItems(result.items);
      setPagination(result.pagination);
      setFilters(result.filters);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load review queue');
    } finally {
      setLoading(false);
    }
  }, [allowed, pagination.page, pagination.pageSize, minConfInput, maxConfInput]);

  useEffect(() => {
    loadQueue();
  }, [loadQueue]);

  const handleApprove = useCallback(async (id: string) => {
    if (actionLoading) return;
    setActionLoading(id);
    try {
      await approveReviewItem(id);
      // Remove from list
      setItems(prev => prev.filter(i => i.id !== id));
      setPagination(prev => ({ ...prev, total: prev.total - 1 }));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to approve item');
    } finally {
      setActionLoading(null);
    }
  }, [actionLoading]);

  const handleReject = useCallback(async (id: string) => {
    if (actionLoading) return;
    setActionLoading(id);
    try {
      await rejectReviewItem(id);
      // Remove from list
      setItems(prev => prev.filter(i => i.id !== id));
      setPagination(prev => ({ ...prev, total: prev.total - 1 }));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to reject item');
    } finally {
      setActionLoading(null);
    }
  }, [actionLoading]);

  const toggleExpanded = useCallback((id: string) => {
    setExpandedIds(prev => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const applyFilters = useCallback(() => {
    setPagination(p => ({ ...p, page: 1 }));
    loadQueue();
  }, [loadQueue]);

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
                The Review Queue is available on Growth plans and above. Current plan: {plan || 'Starter'}.
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
      <div className="flex flex-col lg:flex-row lg:items-center lg:justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-semibold text-gray-900">Review Queue</h1>
            <span className="inline-flex items-center rounded-full bg-primary-100 px-2.5 py-0.5 text-xs font-medium text-primary-700">
              {pagination.total} items
            </span>
          </div>
          <p className="mt-1 text-sm text-gray-500">
            Approve or reject fuzzy matches (40-80% confidence) to improve reconciliation accuracy
          </p>
        </div>
        <button
          onClick={loadQueue}
          disabled={loading}
          className="inline-flex items-center gap-2 rounded-lg border border-gray-200 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
        >
          <RefreshCw className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
          Refresh
        </button>
      </div>

      {/* Help banner */}
      <div className="rounded-lg border border-blue-100 bg-blue-50 p-4 flex items-start gap-3">
        <HelpCircle className="h-5 w-5 text-blue-600 flex-shrink-0 mt-0.5" />
        <div className="text-sm text-blue-800">
          <p className="font-medium">How Review Queue Works</p>
          <p className="mt-1">
            Items in this queue are potential matches between your expected revenue and network statements,
            but the confidence is below the auto-match threshold. Review each item and approve if the match
            looks correct, or reject if it&apos;s a false positive.
          </p>
        </div>
      </div>

      {/* Filters */}
      <div className="rounded-xl border border-gray-100 bg-white p-4 shadow-sm">
        <div className="flex flex-col sm:flex-row sm:items-end gap-4">
          <div className="flex-1">
            <label htmlFor="confidence-min" className="block text-xs font-medium text-gray-600 mb-1">
              Confidence Range
            </label>
            <div className="flex items-center gap-2">
              <div className="flex-1">
                <input
                  id="confidence-min"
                  type="range"
                  min="0"
                  max="100"
                  value={minConfInput}
                  onChange={(e) => setMinConfInput(Number(e.target.value))}
                  className="w-full h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer accent-primary-600"
                />
                <div className="flex justify-between text-xs text-gray-400 mt-1">
                  <span>0%</span>
                  <span>Min: {minConfInput}%</span>
                </div>
              </div>
              <span className="text-gray-400">to</span>
              <div className="flex-1">
                <input
                  type="range"
                  min="0"
                  max="100"
                  value={maxConfInput}
                  onChange={(e) => setMaxConfInput(Number(e.target.value))}
                  className="w-full h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer accent-primary-600"
                />
                <div className="flex justify-between text-xs text-gray-400 mt-1">
                  <span>Max: {maxConfInput}%</span>
                  <span>100%</span>
                </div>
              </div>
            </div>
          </div>
          <button
            onClick={applyFilters}
            className="inline-flex items-center gap-2 rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700"
          >
            <Zap className="h-4 w-4" />
            Apply Filters
          </button>
        </div>
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

      {/* Items List */}
      {loading ? (
        <div className="flex items-center justify-center p-12">
          <Loader2 className="h-6 w-6 animate-spin text-gray-400" />
        </div>
      ) : items.length === 0 ? (
        <div className="rounded-xl border border-gray-100 bg-white p-12 text-center shadow-sm">
          <div className="mx-auto h-12 w-12 rounded-full bg-green-100 flex items-center justify-center">
            <Check className="h-6 w-6 text-green-600" />
          </div>
          <h3 className="mt-4 text-lg font-medium text-gray-900">All caught up!</h3>
          <p className="mt-2 text-sm text-gray-500">
            No items in the review queue matching your criteria. Check back later or adjust the confidence range.
          </p>
        </div>
      ) : (
        <div className="space-y-4">
          {items.map(item => (
            <ReviewItemCard
              key={item.id}
              item={item}
              onApprove={() => handleApprove(item.id)}
              onReject={() => handleReject(item.id)}
              loading={actionLoading === item.id}
              expanded={expandedIds.has(item.id)}
              onToggle={() => toggleExpanded(item.id)}
            />
          ))}
        </div>
      )}

      {/* Pagination */}
      {pagination.totalPages > 1 && (
        <div className="flex items-center justify-between border-t border-gray-100 pt-4">
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
  );
}
