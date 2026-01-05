'use client'

import Link from 'next/link'
import { useRouter } from 'next/navigation'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { placementApi, adapterApi } from '@/lib/api'
import { formatNumber } from '@/lib/utils'
import type { Adapter } from '@/types'
import {
  ArrowLeft,
  Edit,
  Trash2,
  Play,
  Pause,
  AlertCircle,
  CheckCircle,
  Settings,
  BarChart3,
  Users,
} from 'lucide-react'
import { useState } from 'react'

interface PageProps {
  params: { id: string }
}

export default function PlacementDetailPage({ params }: PageProps) {
  const { id } = params
  const router = useRouter()
  const queryClient = useQueryClient()
  const [deleteConfirm, setDeleteConfirm] = useState(false)
  const [confirmText, setConfirmText] = useState('')
  const CONFIRM_KEYWORD = 'DELETE'

  const { data: placement, isLoading } = useQuery({
    queryKey: ['placement', id],
    queryFn: async () => {
      return await placementApi.get(id)
    },
  })

  const { data: adapters, isLoading: adaptersLoading, isError: adaptersError, error: adaptersErrorObj } = useQuery<
    Adapter[],
    Error
  >({
    queryKey: ['adapters', id],
    queryFn: async () => {
      const { data } = await adapterApi.list(id)
      return data
    },
    enabled: !!id,
  })

  const updateMutation = useMutation({
    mutationFn: async (status: 'active' | 'paused') => {
      const { data } = await placementApi.update(id, { status })
      return data
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['placement', id] })
      queryClient.invalidateQueries({ queryKey: ['placements'] })
    },
  })

  const deleteMutation = useMutation({
    mutationFn: async () => {
      await placementApi.delete(id)
    },
    onSuccess: () => {
      router.push('/placements')
    },
  })

  if (isLoading) {
    return <PlacementDetailSkeleton />
  }

  if (!placement) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center">
        <div className="text-center">
          <AlertCircle className="h-12 w-12 text-gray-400 mx-auto mb-3" />
          <h2 className="text-lg font-semibold text-gray-900">Placement not found</h2>
          <p className="text-sm text-gray-600 mt-1">The requested placement does not exist.</p>
          <Link href="/placements" className="btn btn-primary mt-4">
            Back to Placements
          </Link>
        </div>
      </div>
    )
  }

  const statusColors = {
    active: 'bg-success-100 text-success-700',
    paused: 'bg-warning-100 text-warning-700',
    archived: 'bg-gray-100 text-gray-700',
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <Link
            href="/placements"
            className="inline-flex items-center gap-2 text-sm text-gray-600 hover:text-gray-900 mb-4"
          >
            <ArrowLeft className="h-4 w-4" aria-hidden="true" />
            Back to Placements
          </Link>
          <div className="flex items-start justify-between gap-4">
            <div>
              <div className="flex items-center gap-3 mb-2">
                <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">{placement.name}</h1>
                <span className={`badge ${statusColors[placement.status]}`}>
                  {placement.status}
                </span>
              </div>
              <p className="text-sm text-gray-600 font-mono">{placement.id}</p>
            </div>
            <div className="flex items-center gap-2">
              {placement.status === 'active' ? (
                <button
                  onClick={() => updateMutation.mutate('paused')}
                  disabled={updateMutation.isPending}
                  className="btn btn-outline flex items-center gap-2"
                >
                  <Pause className="h-4 w-4" aria-hidden="true" />
                  Pause
                </button>
              ) : (
                <button
                  onClick={() => updateMutation.mutate('active')}
                  disabled={updateMutation.isPending}
                  className="btn btn-success flex items-center gap-2"
                >
                  <Play className="h-4 w-4" aria-hidden="true" />
                  Activate
                </button>
              )}
              <Link href={`/placements/${id}/edit`} className="btn btn-outline flex items-center gap-2">
                <Edit className="h-4 w-4" aria-hidden="true" />
                Edit
              </Link>
              {!deleteConfirm ? (
                <button
                  onClick={() => setDeleteConfirm(true)}
                  className="btn btn-danger flex items-center gap-2"
                >
                  <Trash2 className="h-4 w-4" aria-hidden="true" />
                  Delete
                </button>
              ) : (
                <button
                  onClick={() => deleteMutation.mutate()}
                  disabled={deleteMutation.isPending}
                  className="btn btn-danger animate-pulse"
                >
                  {deleteMutation.isPending ? 'Deleting...' : 'Confirm Delete'}
                </button>
              )}
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-6">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <InfoCard icon={Settings} label="Type" value={placement.type} badge />
          <InfoCard icon={BarChart3} label="Format" value={placement.format} />
          <InfoCard icon={Users} label="Platform" value={placement.platformId} mono />
        </div>

        <section className="card">
          <h2 className="text-xl font-semibold text-gray-900 mb-4">Adapter Configuration</h2>
          {adaptersLoading ? (
            <div className="p-4 text-gray-600">Loading adapters…</div>
          ) : adaptersError ? (
            <div className="p-4 bg-red-50 border border-red-200 rounded text-sm text-red-700">
              Failed to load adapters{adaptersErrorObj instanceof Error ? `: ${adaptersErrorObj.message}` : ''}
            </div>
          ) : adapters && adapters.length > 0 ? (
            <div className="space-y-3">
              {adapters.map((adapter) => (
                <div
                  key={adapter.id}
                  className="flex items-center justify-between p-4 border border-gray-200 rounded-lg hover:border-primary-200 transition-colors"
                >
                  <div className="flex items-center gap-4">
                    <div
                      className={`h-10 w-10 rounded-lg flex items-center justify-center font-semibold text-sm ${
                        adapter.status === 'active'
                          ? 'bg-success-50 text-success-700'
                          : 'bg-gray-100 text-gray-500'
                      }`}
                    >
                      #{adapter.priority}
                    </div>
                    <div>
                      <h3 className="font-semibold text-gray-900">{adapter.name}</h3>
                      <p className="text-xs text-gray-500">{adapter.network}</p>
                    </div>
                  </div>
                  <div className="flex items-center gap-6">
                    <div className="text-right">
                      <div className="text-xs text-gray-500">eCPM</div>
                      <div className="font-semibold text-gray-900">${adapter.ecpm?.toFixed(2) || '0.00'}</div>
                    </div>
                    <div className="text-right">
                      <div className="text-xs text-gray-500">Fill Rate</div>
                      <div className="font-semibold text-gray-900">{((adapter.fillRate || 0) * 100).toFixed(1)}%</div>
                    </div>
                    <div className="text-right">
                      <div className="text-xs text-gray-500">Impressions</div>
                      <div className="font-semibold text-gray-900">{formatNumber(adapter.impressionCount)}</div>
                    </div>
                    <Link
                      href={`/adapters/${adapter.id}`}
                      className="btn btn-outline btn-sm"
                    >
                      Configure
                    </Link>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="text-center py-8 text-gray-500">
              <Settings className="h-12 w-12 mx-auto mb-3 text-gray-300" />
              <p className="text-sm">No adapters configured yet</p>
              <Link href={`/placements/${id}/adapters/new`} className="btn btn-primary mt-4">
                Add First Adapter
              </Link>
            </div>
          )}
        </section>

        <section className="card">
          <h2 className="text-xl font-semibold text-gray-900 mb-4">Metadata</h2>
          <dl className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <dt className="text-gray-500">Created</dt>
              <dd className="font-medium text-gray-900">{new Date(placement.createdAt).toLocaleString()}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Last Updated</dt>
              <dd className="font-medium text-gray-900">{new Date(placement.updatedAt).toLocaleString()}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Publisher ID</dt>
              <dd className="font-mono text-xs text-gray-700" title={placement.publisherId}>
                {maskId(placement.publisherId)}
              </dd>
            </div>
          </dl>
        </section>

        {deleteConfirm && (
          <div role="dialog" aria-modal="true" className="fixed inset-0 z-50 flex items-center justify-center">
            <div
              className="absolute inset-0 bg-black/30"
              role="button"
              aria-label="Close confirmation dialog"
              tabIndex={0}
              onClick={() => setDeleteConfirm(false)}
              onKeyDown={(e) => {
                if (e.key === 'Escape' || e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  setDeleteConfirm(false)
                }
              }}
            />
            <div className="relative bg-white rounded-lg shadow-lg max-w-md w-full p-6">
              <h3 className="text-lg font-semibold text-gray-900">Confirm deletion</h3>
              <p className="text-sm text-gray-600 mt-2">
                This will permanently remove the placement &quot;{placement.name}&quot; and its adapter configuration links. Type <span className="font-mono">{CONFIRM_KEYWORD}</span> to confirm.
              </p>
              <input
                aria-label="Confirmation keyword"
                className="input mt-4 w-full"
                value={confirmText}
                onChange={(e) => setConfirmText(e.target.value.toUpperCase())}
                placeholder={CONFIRM_KEYWORD}
              />
              <div className="mt-6 flex items-center justify-end gap-3">
                <button className="btn" onClick={() => setDeleteConfirm(false)} disabled={deleteMutation.isPending}>
                  Cancel
                </button>
                <button
                  className="btn btn-danger"
                  onClick={() => deleteMutation.mutate()}
                  disabled={deleteMutation.isPending || confirmText !== CONFIRM_KEYWORD}
                >
                  {deleteMutation.isPending ? 'Deleting…' : 'Delete'}
                </button>
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  )
}

function InfoCard({
  icon: Icon,
  label,
  value,
  badge,
  mono,
}: {
  icon: any
  label: string
  value: string
  badge?: boolean
  mono?: boolean
}) {
  return (
    <div className="card">
      <div className="flex items-center gap-3 mb-2">
        <div className="h-10 w-10 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center">
          <Icon className="h-5 w-5" aria-hidden="true" />
        </div>
        <span className="text-sm font-medium text-gray-500 uppercase tracking-wide">{label}</span>
      </div>
      {badge ? (
        <span className="badge badge-info capitalize text-sm">{value}</span>
      ) : (
        <span className={`text-lg font-semibold text-gray-900 ${mono ? 'font-mono text-sm' : ''}`}>
          {value}
        </span>
      )}
    </div>
  )
}

function maskId(id: string): string {
  if (!id) return ''
  const visible = id.slice(-4)
  return `••••••••-${visible}`
}

function PlacementDetailSkeleton() {
  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="animate-pulse space-y-4">
            <div className="h-4 bg-gray-200 rounded w-32"></div>
            <div className="h-8 bg-gray-200 rounded w-64"></div>
            <div className="h-4 bg-gray-200 rounded w-96"></div>
          </div>
        </div>
      </header>
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="card animate-pulse">
          <div className="h-64 bg-gray-200 rounded"></div>
        </div>
      </main>
    </div>
  )
}
