'use client'

import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { RefreshCcw } from 'lucide-react'
import { TransparencyApiError, transparencyApi, type TransparencySummary } from '@/lib/transparency'
import { isFeatureEnabled } from '@/lib/featureFlags'

export default function TransparencySummaryPage() {
  const canManuallyRefresh = isFeatureEnabled('transparencyRefresh')
  const {
    data,
    error,
    isLoading,
    isFetching,
    refetch,
    dataUpdatedAt,
  } = useQuery<TransparencySummary, Error>({
    queryKey: ['transparency-summary'],
    queryFn: ({ signal }) => transparencyApi.summary({ signal }),
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: 0,
    staleTime: 60_000,
  })

  const lastUpdatedLabel = useMemo(() => {
    if (!dataUpdatedAt) return null
    return new Date(dataUpdatedAt).toLocaleString()
  }, [dataUpdatedAt])

  const winnerList = data?.winners_by_source ?? []
  const isDisabled =
    error instanceof TransparencyApiError &&
    error.status === 503 &&
    /disabled/i.test(error.message)

  return (
    <div className="p-6 space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold text-gray-900">Transparency — Summary</h1>
          <p className="text-sm text-gray-500">Live sampling metrics across cryptographically verified auctions.</p>
        </div>
        <div className="flex items-center gap-3 text-sm text-gray-500">
          <span>
            Last updated: {lastUpdatedLabel ?? '—'}
          </span>
          {canManuallyRefresh ? (
            <button
              type="button"
              onClick={() => refetch()}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 border rounded text-gray-700 hover:bg-gray-50 disabled:opacity-50"
              disabled={isFetching}
            >
              <RefreshCcw className="h-4 w-4" aria-hidden="true" />
              Refresh
            </button>
          ) : (
            <span className="text-xs text-gray-400" role="status">
              Manual refresh disabled
            </span>
          )}
        </div>
      </div>

      {isLoading && <SummarySkeleton />}

      {!isLoading && isDisabled && (
        <div className="border border-gray-200 rounded-lg bg-gray-50 px-4 py-3" role="status" aria-live="polite">
          <p className="text-sm text-gray-900 font-semibold">Transparency is currently disabled</p>
          <p className="text-xs text-gray-700 mt-1">
            This environment has the Transparency API turned off. Other console features should continue to work normally.
          </p>
        </div>
      )}

      {!isLoading && error && !isDisabled && (
        <div className="border border-red-200 rounded-lg bg-red-50 px-4 py-3" role="alert" aria-live="assertive">
          <p className="text-sm text-red-900 font-semibold">Unable to load transparency summary</p>
          <p className="text-xs text-red-800 mt-1">{error.message}</p>
          {canManuallyRefresh && (
            <button
              type="button"
              onClick={() => refetch()}
              className="mt-3 inline-flex items-center gap-1.5 text-sm font-medium text-red-900"
            >
              <RefreshCcw className="h-4 w-4" aria-hidden="true" />
              Retry
            </button>
          )}
        </div>
      )}

      {!isLoading && !error && data && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div className="border rounded-lg p-4 space-y-2">
            <SummaryStat label="Total sampled auctions" value={data.total_sampled.toLocaleString()} />
            <SummaryStat label="Average fee (bp)" value={data.avg_fee_bp.toFixed(2)} />
            <SummaryStat label="Avg. publisher share" value={`${(data.publisher_share_avg * 100).toFixed(2)}%`} />
          </div>
          <div className="border rounded-lg p-4">
            <div className="flex items-center justify-between mb-3">
              <h2 className="font-semibold text-gray-900">Winners by source</h2>
              <span className="text-xs text-gray-500">Top contributors</span>
            </div>
            <ul className="text-sm text-gray-700 space-y-1">
              {winnerList.length > 0 ? (
                winnerList.map((winner) => (
                  <li key={winner.source} className="flex items-center justify-between">
                    <span className="font-medium text-gray-900">{winner.source}</span>
                    <span className="text-gray-600">{winner.count.toLocaleString()}</span>
                  </li>
                ))
              ) : (
                <li className="text-gray-500">No sampling data available yet.</li>
              )}
            </ul>
          </div>
        </div>
      )}
    </div>
  )
}

function SummaryStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-gray-500">{label}</span>
      <span className="text-base font-semibold text-gray-900">{value}</span>
    </div>
  )
}

function SummarySkeleton() {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
      <div className="border rounded-lg p-4 space-y-3 animate-pulse">
        <div className="h-4 bg-gray-200 rounded w-1/2" />
        <div className="h-4 bg-gray-200 rounded w-1/3" />
        <div className="h-4 bg-gray-200 rounded w-1/4" />
      </div>
      <div className="border rounded-lg p-4 space-y-3 animate-pulse">
        <div className="h-5 bg-gray-200 rounded w-1/3" />
        <div className="h-4 bg-gray-200 rounded w-2/3" />
        <div className="h-4 bg-gray-200 rounded w-1/2" />
        <div className="h-4 bg-gray-200 rounded w-1/4" />
      </div>
    </div>
  )
}
