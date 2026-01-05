'use client'

import { useCallback, useMemo, useState } from 'react'
import Link from 'next/link'
import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query'
import { adapterApi, placementApi } from '@/lib/api'
import { formatNumber } from '@/lib/utils'
import {
  Plus,
  Search,
  Filter,
  SlidersHorizontal,
  Signal,
  Zap,
  Loader2,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type { Adapter, Placement } from '@/types'

export default function AdaptersPage() {
  const [search, setSearch] = useState('')
  const [statusFilter, setStatusFilter] = useState<'all' | Adapter['status']>('all')
  const [networkFilter, setNetworkFilter] = useState('all')
  const queryClient = useQueryClient()

  const {
    data: adapters,
    isLoading,
    isError: isAdaptersError,
    error: adaptersError,
    refetch: refetchAdapters,
  } = useQuery<Adapter[], Error>({
    queryKey: ['adapters'],
    queryFn: async () => {
      const { data } = await adapterApi.list()
      return data
    },
    staleTime: 60_000,
    retry: 1,
  })

  const {
    data: placements,
    isError: isPlacementsError,
    error: placementsError,
    refetch: refetchPlacements,
  } = useQuery<Placement[], Error>({
    queryKey: ['placements', 'lookup'],
    queryFn: async () => {
      const response = await placementApi.list({ page: 1, pageSize: 200 })
      return response.data
    },
    placeholderData: keepPreviousData,
    staleTime: 5 * 60 * 1000,
    gcTime: 30 * 60 * 1000,
    retry: 1,
  })

  const handlePrefetch = useCallback(
    (adapter: Adapter) => {
      queryClient.prefetchQuery({
        queryKey: ['adapter', adapter.id],
        queryFn: async () => {
          const { data } = await adapterApi.get(adapter.id)
          return data
        },
        staleTime: 60_000,
      })

      if (adapter.placementId) {
        queryClient.prefetchQuery({
          queryKey: ['placement', adapter.placementId],
          queryFn: async () => {
            return await placementApi.get(adapter.placementId)
          },
          staleTime: 5 * 60 * 1000,
        })
      }
    },
    [queryClient]
  )

  const filteredAdapters = useMemo(() => {
    if (!Array.isArray(adapters)) return []

    return adapters.filter((adapter) => {
      const matchesSearch = search
        ? adapter.name.toLowerCase().includes(search.toLowerCase()) ||
          adapter.network.toLowerCase().includes(search.toLowerCase()) ||
          adapter.id.toLowerCase().includes(search.toLowerCase())
        : true

      const matchesStatus = statusFilter === 'all' || adapter.status === statusFilter
      const matchesNetwork = networkFilter === 'all' || adapter.network === networkFilter

      return matchesSearch && matchesStatus && matchesNetwork
    })
  }, [adapters, networkFilter, search, statusFilter])

  const networks = useMemo(() => {
    if (!Array.isArray(adapters)) return []
    return Array.from(new Set(adapters.map((adapter) => adapter.network))).sort()
  }, [adapters])

  const adaptersErrorMessage = adaptersError?.message || 'Please check your connection and try again.'
  const placementLookupError = isPlacementsError
    ? placementsError?.message || 'Placement names are temporarily unavailable.'
    : null

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div>
              <p className="text-sm font-medium text-primary-600">Network Integrations</p>
              <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Adapters</h1>
              <p className="text-sm text-gray-600 mt-1">
                Manage mediated networks, priorities, and performance benchmarks.
              </p>
            </div>
            <Link href="/adapters/new" className="btn btn-primary flex items-center gap-2">
              <Plus className="h-4 w-4" aria-hidden="true" />
              Add Adapter
            </Link>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-6">
        <div className="card">
          <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
            <div className="relative w-full lg:max-w-md">
              <Search className="absolute left-3 top-1/2 h-5 w-5 -translate-y-1/2 text-gray-400" aria-hidden="true" />
              <input
                type="search"
                placeholder="Search adapters by name, network, or ID"
                className="input pl-10"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <div className="flex items-center gap-2">
                <Filter className="h-5 w-5 text-gray-500" aria-hidden="true" />
                <select
                  value={statusFilter}
                  onChange={(event) => setStatusFilter(event.target.value as typeof statusFilter)}
                  className="input w-40"
                >
                  <option value="all">All Statuses</option>
                  <option value="active">Active</option>
                  <option value="inactive">Inactive</option>
                  <option value="testing">Testing</option>
                </select>
              </div>
              <div className="flex items-center gap-2">
                <SlidersHorizontal className="h-5 w-5 text-gray-500" aria-hidden="true" />
                <select
                  value={networkFilter}
                  onChange={(event) => setNetworkFilter(event.target.value)}
                  className="input w-48"
                >
                  <option value="all">All Networks</option>
                  {networks.map((network) => (
                    <option key={network} value={network}>
                      {network}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          </div>
        </div>

        {placementLookupError && (
          <InlineNotice
            message={`Placement lookup failed: ${placementLookupError}`}
            actionLabel="Retry lookup"
            onAction={() => refetchPlacements()}
          />
        )}

        {isLoading && <AdaptersSkeleton />}

        {!isLoading && isAdaptersError && (
          <ErrorState
            title="We couldn’t load your adapters"
            description={adaptersErrorMessage}
            onRetry={() => refetchAdapters()}
          />
        )}

        {!isLoading && !isAdaptersError && filteredAdapters.length > 0 && (
          <div className="grid grid-cols-1 gap-4">
            {filteredAdapters.map((adapter) => (
              <AdapterCard
                key={adapter.id}
                adapter={adapter}
                placements={placements}
                onPrefetch={handlePrefetch}
              />
            ))}
          </div>
        )}

        {!isLoading && !isAdaptersError && filteredAdapters.length === 0 && <EmptyState />}
      </main>
    </div>
  )
}

function AdapterCard({
  adapter,
  placements,
  onPrefetch,
}: {
  adapter: Adapter
  placements?: Placement[]
  onPrefetch?: (adapter: Adapter) => void
}) {
  const statusChip = {
    active: 'bg-success-100 text-success-700',
    inactive: 'bg-gray-100 text-gray-700',
    testing: 'bg-warning-100 text-warning-700',
  }[adapter.status]

  const placementName = placements?.find((placement) => placement.id === adapter.placementId)?.name

  return (
    <div className="card hover:shadow-lg transition-shadow duration-200">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <div className="space-y-2">
          <div className="flex items-center gap-3">
            <div className="h-10 w-10 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center font-semibold">
              {adapter.network.slice(0, 2).toUpperCase()}
            </div>
            <div>
              <h3 className="text-lg font-semibold text-gray-900">{adapter.name}</h3>
              <p className="text-xs font-mono text-gray-500">{adapter.id}</p>
            </div>
            <span className={`badge capitalize ${statusChip}`}>{adapter.status}</span>
          </div>
          <div className="text-sm text-gray-600">
            Network <span className="font-medium text-gray-900">{adapter.network}</span>
            {placementName && (
              <>
                {' · '}Placement <span className="font-medium text-gray-900">{placementName}</span>
              </>
            )}
          </div>
        </div>
        <div className="flex flex-wrap justify-end gap-6">
          <Metric label="Priority" value={`#${adapter.priority}`} icon={Signal} />
          <Metric label="eCPM" value={`$${adapter.ecpm?.toFixed(2) || '0.00'}`} icon={Zap} />
          <Metric label="Fill Rate" value={`${((adapter.fillRate || 0) * 100).toFixed(1)}%`} icon={Loader2} />
          <Metric label="Requests" value={formatNumber(adapter.requestCount)} />
          <Link
            href={`/adapters/${adapter.id}`}
            className="btn btn-outline self-center"
            onMouseEnter={() => onPrefetch?.(adapter)}
            onFocus={() => onPrefetch?.(adapter)}
          >
            Manage
          </Link>
        </div>
      </div>
    </div>
  )
}

function Metric({ label, value, icon: Icon }: { label: string; value: string; icon?: LucideIcon }) {
  return (
    <div className="text-right">
      <div className="flex items-center justify-end gap-1 text-xs uppercase tracking-wide text-gray-500">
        {Icon && <Icon className="h-3 w-3" aria-hidden={true} />}
        <span>{label}</span>
      </div>
      <div className="text-lg font-semibold text-gray-900">{value}</div>
    </div>
  )
}

function EmptyState() {
  return (
    <div className="card text-center py-12">
      <div className="mx-auto h-16 w-16 rounded-full bg-gray-100 flex items-center justify-center mb-4">
        <Plus className="h-8 w-8 text-gray-400" aria-hidden="true" />
      </div>
      <h3 className="text-lg font-semibold text-gray-900 mb-2">No adapters connected</h3>
      <p className="text-sm text-gray-600 mb-6 max-w-md mx-auto">
        Connect your demand partners to unlock real-time competition and maximize yield across your placements.
      </p>
      <Link href="/adapters/new" className="btn btn-primary inline-flex items-center gap-2">
        <Plus className="h-4 w-4" aria-hidden="true" />
        Add Adapter
      </Link>
    </div>
  )
}

function InlineNotice({
  message,
  actionLabel,
  onAction,
}: {
  message: string
  actionLabel: string
  onAction: () => void
}) {
  return (
    <div className="border border-amber-200 rounded-lg bg-amber-50 px-4 py-3 flex flex-col gap-2 text-amber-900">
      <p className="text-sm font-medium">{message}</p>
      <button
        type="button"
        className="self-start text-sm font-semibold text-amber-900 underline"
        onClick={onAction}
      >
        {actionLabel}
      </button>
    </div>
  )
}

function ErrorState({
  title,
  description,
  onRetry,
}: {
  title: string
  description: string
  onRetry: () => void
}) {
  return (
    <div className="card border border-red-200 bg-red-50 text-red-900">
      <div className="space-y-2">
        <h3 className="text-lg font-semibold">{title}</h3>
        <p className="text-sm">{description}</p>
        <button type="button" className="btn btn-danger" onClick={onRetry}>
          Try again
        </button>
      </div>
    </div>
  )
}

const SKELETON_CARD_KEYS = ['card-1', 'card-2', 'card-3', 'card-4'] as const
const SKELETON_METRIC_KEYS = ['metric-1', 'metric-2', 'metric-3', 'metric-4'] as const

function AdaptersSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-4">
      {SKELETON_CARD_KEYS.map((cardKey) => (
        <div key={cardKey} className="card">
          <div className="animate-pulse space-y-4">
            <div className="flex items-center gap-3">
              <div className="h-10 w-10 rounded-lg bg-gray-200" />
              <div className="space-y-2">
                <div className="h-5 w-40 rounded bg-gray-200" />
                <div className="h-3 w-48 rounded bg-gray-200" />
              </div>
            </div>
            <div className="flex flex-wrap justify-between gap-4">
              {SKELETON_METRIC_KEYS.map((metricKey) => (
                <div key={metricKey} className="h-10 w-24 rounded bg-gray-200" />
              ))}
            </div>
          </div>
        </div>
      ))}
    </div>
  )
}
