'use client';

import { useEffect, useState, use, useCallback } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import {
  AlertCircle,
  ArrowLeft,
  CheckCircle2,
  Clock,
  DollarSign,
  ExternalLink,
  FileText,
  Loader2,
  Send,
  XCircle,
} from 'lucide-react';
import type { DisputeRecord, DisputeStatus } from '@/lib/vra';
import { getDispute, submitDispute, cancelDispute } from '@/lib/vra';
import { usePlanAccess } from '@/lib/usePlanAccess';

const STATUS_CONFIG: Record<DisputeStatus, { label: string; color: string; icon: typeof Clock; bgColor: string }> = {
  draft: { label: 'Draft', color: 'text-gray-700', bgColor: 'bg-gray-100', icon: FileText },
  submitted: { label: 'Submitted', color: 'text-blue-700', bgColor: 'bg-blue-100', icon: Send },
  in_progress: { label: 'In Progress', color: 'text-yellow-700', bgColor: 'bg-yellow-100', icon: Clock },
  resolved: { label: 'Resolved', color: 'text-green-700', bgColor: 'bg-green-100', icon: CheckCircle2 },
  rejected: { label: 'Rejected', color: 'text-red-700', bgColor: 'bg-red-100', icon: XCircle },
  cancelled: { label: 'Cancelled', color: 'text-gray-500', bgColor: 'bg-gray-100', icon: XCircle },
};

function StatusTimeline({ dispute }: { dispute: DisputeRecord }) {
  const steps: { label: string; date: string | null; status: 'completed' | 'current' | 'pending' }[] = [
    { label: 'Created', date: dispute.createdAt, status: 'completed' },
    {
      label: 'Submitted',
      date: dispute.submittedAt ?? null,
      status: dispute.submittedAt ? 'completed' : dispute.status === 'draft' ? 'current' : 'pending',
    },
    {
      label: 'Resolved',
      date: dispute.resolvedAt ?? null,
      status: dispute.resolvedAt
        ? 'completed'
        : ['submitted', 'in_progress'].includes(dispute.status)
          ? 'current'
          : 'pending',
    },
  ];

  return (
    <div className="flex items-center justify-between">
      {steps.map((step, idx) => (
        <div key={step.label} className="flex items-center">
          <div className="flex flex-col items-center">
            <div
              className={`flex h-8 w-8 items-center justify-center rounded-full border-2 ${
                step.status === 'completed'
                  ? 'border-green-500 bg-green-50 text-green-600'
                  : step.status === 'current'
                    ? 'border-primary-500 bg-primary-50 text-primary-600'
                    : 'border-gray-200 bg-white text-gray-400'
              }`}
            >
              {step.status === 'completed' ? (
                <CheckCircle2 className="h-4 w-4" />
              ) : (
                <span className="text-xs font-medium">{idx + 1}</span>
              )}
            </div>
            <span className="mt-2 text-xs font-medium text-gray-600">{step.label}</span>
            {step.date && (
              <span className="text-xs text-gray-400">
                {new Date(step.date).toLocaleDateString()}
              </span>
            )}
          </div>
          {idx < steps.length - 1 && (
            <div
              className={`h-0.5 w-16 sm:w-24 mx-2 ${
                steps[idx + 1].status === 'completed' || step.status === 'completed'
                  ? 'bg-green-300'
                  : 'bg-gray-200'
              }`}
            />
          )}
        </div>
      ))}
    </div>
  );
}

export default function DisputeDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const router = useRouter();
  const { allowed, isLoading: planLoading } = usePlanAccess(['growth', 'scale', 'enterprise']);
  
  const [dispute, setDispute] = useState<DisputeRecord | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState(false);

  const loadDisputeCallback = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await getDispute(id);
      setDispute(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load dispute');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    if (!allowed || !id) {
      setLoading(false);
      return;
    }
    loadDisputeCallback();
  }, [allowed, id, loadDisputeCallback]);

  const handleSubmit = useCallback(async () => {
    if (!dispute || actionLoading) return;
    setActionLoading(true);
    try {
      await submitDispute(dispute.id);
      await loadDisputeCallback();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to submit dispute');
    } finally {
      setActionLoading(false);
    }
  }, [dispute, actionLoading, loadDisputeCallback]);

  const handleCancel = useCallback(async () => {
    if (!dispute || actionLoading) return;
    if (!confirm('Are you sure you want to cancel this dispute? This action cannot be undone.')) return;
    setActionLoading(true);
    try {
      await cancelDispute(dispute.id);
      await loadDisputeCallback();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to cancel dispute');
    } finally {
      setActionLoading(false);
    }
  }, [dispute, actionLoading, loadDisputeCallback]);

  if (planLoading || loading) {
    return (
      <div className="p-6 flex items-center justify-center min-h-[400px]">
        <Loader2 className="h-6 w-6 animate-spin text-gray-400" />
      </div>
    );
  }

  if (!allowed) {
    return (
      <div className="p-6">
        <div className="rounded-xl border border-amber-200 bg-amber-50 p-6">
          <p className="font-semibold text-amber-900">Upgrade required to view disputes.</p>
        </div>
      </div>
    );
  }

  if (error || !dispute) {
    return (
      <div className="p-6">
        <div className="rounded-xl border border-red-200 bg-red-50 p-6 flex items-start gap-3">
          <AlertCircle className="h-5 w-5 text-red-600 flex-shrink-0" />
          <div>
            <p className="font-semibold text-red-800">Error loading dispute</p>
            <p className="mt-1 text-sm text-red-700">{error || 'Dispute not found'}</p>
            <Link href="/recon/disputes" className="mt-3 inline-flex items-center gap-1 text-sm text-red-600 hover:underline">
              <ArrowLeft className="h-4 w-4" />
              Back to Disputes
            </Link>
          </div>
        </div>
      </div>
    );
  }

  const statusConfig = STATUS_CONFIG[dispute.status];
  const StatusIcon = statusConfig.icon;

  return (
    <div className="p-6 space-y-6 max-w-4xl mx-auto">
      {/* Back link */}
      <Link
        href="/recon/disputes"
        className="inline-flex items-center gap-1.5 text-sm text-gray-600 hover:text-gray-900"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to Disputes
      </Link>

      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-start sm:justify-between gap-4">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-semibold text-gray-900">Dispute Details</h1>
            <span className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-sm font-medium ${statusConfig.bgColor} ${statusConfig.color}`}>
              <StatusIcon className="h-4 w-4" />
              {statusConfig.label}
            </span>
          </div>
          <p className="mt-1 text-sm text-gray-500 font-mono">{dispute.disputeId}</p>
        </div>
        <div className="flex items-center gap-3">
          {dispute.status === 'draft' && (
            <>
              <button
                onClick={handleSubmit}
                disabled={actionLoading}
                className="inline-flex items-center gap-2 rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700 disabled:opacity-50"
              >
                {actionLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
                Submit to Network
              </button>
              <button
                onClick={handleCancel}
                disabled={actionLoading}
                className="inline-flex items-center gap-2 rounded-lg border border-gray-200 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
              >
                Cancel Dispute
              </button>
            </>
          )}
          {(dispute.status === 'submitted' || dispute.status === 'in_progress') && (
            <button
              onClick={handleCancel}
              disabled={actionLoading}
              className="inline-flex items-center gap-2 rounded-lg border border-red-200 px-4 py-2 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-50"
            >
              {actionLoading && <Loader2 className="h-4 w-4 animate-spin" />}
              Withdraw Dispute
            </button>
          )}
        </div>
      </div>

      {/* Timeline */}
      <div className="rounded-xl border border-gray-100 bg-white p-6 shadow-sm">
        <h2 className="text-sm font-semibold text-gray-900 mb-6">Progress</h2>
        <StatusTimeline dispute={dispute} />
      </div>

      {/* Details Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Dispute Info */}
        <div className="rounded-xl border border-gray-100 bg-white p-6 shadow-sm">
          <h2 className="text-sm font-semibold text-gray-900 mb-4">Dispute Information</h2>
          <dl className="space-y-4">
            <div>
              <dt className="text-xs text-gray-500 uppercase tracking-wide">Network</dt>
              <dd className="mt-1 text-sm font-medium text-gray-900">{dispute.network}</dd>
            </div>
            <div>
              <dt className="text-xs text-gray-500 uppercase tracking-wide">Disputed Amount</dt>
              <dd className="mt-1 flex items-center gap-1.5">
                <DollarSign className="h-4 w-4 text-gray-400" />
                <span className="text-lg font-semibold text-gray-900">
                  {dispute.amount.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                </span>
                <span className="text-sm text-gray-500">USD</span>
              </dd>
            </div>
            {dispute.contact && (
              <div>
                <dt className="text-xs text-gray-500 uppercase tracking-wide">Contact</dt>
                <dd className="mt-1 text-sm text-gray-900">{dispute.contact}</dd>
              </div>
            )}
            {dispute.notes && (
              <div>
                <dt className="text-xs text-gray-500 uppercase tracking-wide">Notes</dt>
                <dd className="mt-1 text-sm text-gray-700 whitespace-pre-wrap">{dispute.notes}</dd>
              </div>
            )}
            {dispute.evidenceUri && (
              <div>
                <dt className="text-xs text-gray-500 uppercase tracking-wide">Evidence</dt>
                <dd className="mt-1">
                  <a
                    href={dispute.evidenceUri}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1 text-sm text-primary-600 hover:text-primary-700"
                  >
                    View Evidence Package
                    <ExternalLink className="h-3.5 w-3.5" />
                  </a>
                </dd>
              </div>
            )}
          </dl>
        </div>

        {/* Resolution Info */}
        <div className="rounded-xl border border-gray-100 bg-white p-6 shadow-sm">
          <h2 className="text-sm font-semibold text-gray-900 mb-4">Resolution</h2>
          {dispute.status === 'resolved' ? (
            <dl className="space-y-4">
              <div>
                <dt className="text-xs text-gray-500 uppercase tracking-wide">Resolution Amount</dt>
                <dd className="mt-1 flex items-center gap-1.5">
                  <CheckCircle2 className="h-5 w-5 text-green-500" />
                  <span className="text-lg font-semibold text-green-600">
                    ${(dispute.resolutionAmount ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 })}
                  </span>
                </dd>
              </div>
              {dispute.networkRefId && (
                <div>
                  <dt className="text-xs text-gray-500 uppercase tracking-wide">Network Reference</dt>
                  <dd className="mt-1 text-sm font-mono text-gray-700">{dispute.networkRefId}</dd>
                </div>
              )}
              {dispute.resolutionNotes && (
                <div>
                  <dt className="text-xs text-gray-500 uppercase tracking-wide">Resolution Notes</dt>
                  <dd className="mt-1 text-sm text-gray-700 whitespace-pre-wrap">{dispute.resolutionNotes}</dd>
                </div>
              )}
              {dispute.resolvedAt && (
                <div>
                  <dt className="text-xs text-gray-500 uppercase tracking-wide">Resolved On</dt>
                  <dd className="mt-1 text-sm text-gray-700">
                    {new Date(dispute.resolvedAt).toLocaleDateString(undefined, {
                      year: 'numeric',
                      month: 'long',
                      day: 'numeric',
                    })}
                  </dd>
                </div>
              )}
            </dl>
          ) : dispute.status === 'rejected' ? (
            <div className="flex flex-col items-center justify-center py-8 text-center">
              <XCircle className="h-12 w-12 text-red-400" />
              <p className="mt-3 text-sm font-medium text-red-700">Dispute Rejected</p>
              {dispute.resolutionNotes && (
                <p className="mt-2 text-sm text-gray-600 max-w-xs">{dispute.resolutionNotes}</p>
              )}
            </div>
          ) : dispute.status === 'cancelled' ? (
            <div className="flex flex-col items-center justify-center py-8 text-center">
              <XCircle className="h-12 w-12 text-gray-300" />
              <p className="mt-3 text-sm font-medium text-gray-600">Dispute Cancelled</p>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center py-8 text-center">
              <Clock className="h-12 w-12 text-gray-300" />
              <p className="mt-3 text-sm font-medium text-gray-600">Pending Resolution</p>
              <p className="mt-1 text-xs text-gray-500">
                The network is reviewing your dispute
              </p>
            </div>
          )}
        </div>
      </div>

      {/* Timestamps */}
      <div className="rounded-xl border border-gray-100 bg-white p-6 shadow-sm">
        <h2 className="text-sm font-semibold text-gray-900 mb-4">Activity</h2>
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 text-sm">
          <div>
            <span className="text-gray-500">Created</span>
            <p className="font-medium text-gray-900">
              {new Date(dispute.createdAt).toLocaleString()}
            </p>
          </div>
          <div>
            <span className="text-gray-500">Last Updated</span>
            <p className="font-medium text-gray-900">
              {new Date(dispute.updatedAt).toLocaleString()}
            </p>
          </div>
          {dispute.submittedAt && (
            <div>
              <span className="text-gray-500">Submitted</span>
              <p className="font-medium text-gray-900">
                {new Date(dispute.submittedAt).toLocaleString()}
              </p>
            </div>
          )}
          {dispute.resolvedAt && (
            <div>
              <span className="text-gray-500">Resolved</span>
              <p className="font-medium text-gray-900">
                {new Date(dispute.resolvedAt).toLocaleString()}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
