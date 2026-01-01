'use client'

import { useEffect, useMemo, useRef, useState } from 'react'
import Link from 'next/link'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { transparencyApi, type TransparencyAuction } from '@/lib/transparency'
import { useDebouncedValue, useUrlQueryParams } from '@/lib/hooks'
import { VerifyBadge, Skeleton, CopyButton } from '@/components/ui'
import { isFeatureEnabled } from '@/lib/featureFlags'
import { usePlanAccess } from '@/lib/usePlanAccess'
import {
  TRANSPARENCY_AUCTION_PAGE_SIZE,
  type AuctionsResponse,
  type TransparencyFilterParams,
  filtersEqual,
  isValidGeo,
  isValidIsoDate,
  normalizeGeo,
} from './filterUtils'

const AUCTIONS_TABLE_SKELETON_KEYS = ['auctions-row-1', 'auctions-row-2', 'auctions-row-3', 'auctions-row-4', 'auctions-row-5']

function formatCurrency(value: number, cur: string) {
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency: cur || 'USD', maximumFractionDigits: 4 }).format(value)
  } catch {
    return `${value.toFixed(4)} ${cur || 'USD'}`
  }
}

export type AuctionsClientProps = {
  initialPage: number
  initialFilters: TransparencyFilterParams
  initialData: AuctionsResponse | null
  initialError?: string | null
}

export function AuctionsClient({ initialPage, initialFilters, initialData, initialError }: AuctionsClientProps) {
  const canManuallyRefresh = isFeatureEnabled('transparencyRefresh')
  const { allowed, isLoading: planLoading, plan, error: planError } = usePlanAccess(['growth', 'scale', 'enterprise'])
  const { params: urlParams, updateParams } = useUrlQueryParams()
  const [page, setPage] = useState(Number.isFinite(initialPage) ? initialPage : 1)
  const [from, setFrom] = useState(initialFilters.from ?? urlParams?.get('from') ?? '')
  const [to, setTo] = useState(initialFilters.to ?? urlParams?.get('to') ?? '')
  const [placementId, setPlacementId] = useState(initialFilters.placement_id ?? urlParams?.get('placement_id') ?? '')
  const [surface, setSurface] = useState(initialFilters.surface ?? urlParams?.get('surface') ?? '')
  const [geo, setGeo] = useState(initialFilters.geo ?? urlParams?.get('geo') ?? '')
  const [fromError, setFromError] = useState<string | null>(null)
  const [toError, setToError] = useState<string | null>(null)
  const [geoError, setGeoError] = useState<string | null>(null)
  const [stickyError, setStickyError] = useState<string | null>(initialError ?? null)

  const debouncedPlacementId = useDebouncedValue(placementId, 300)
  const debouncedSurface = useDebouncedValue(surface, 300)
  const debouncedGeo = useDebouncedValue(geo, 300)

  const handleFromChange = (value: string) => {
    setFrom(value)
    setFromError(value && !isValidIsoDate(value) ? 'Use ISO-8601 format (e.g., 2024-05-01T00:00:00Z).' : null)
  }

  const handleToChange = (value: string) => {
    setTo(value)
    setToError(value && !isValidIsoDate(value) ? 'Use ISO-8601 format (e.g., 2024-05-07T23:59:59Z).' : null)
  }

  const handleGeoChange = (value: string) => {
    const normalized = normalizeGeo(value)
    setGeo(normalized)
    setGeoError(normalized && !isValidGeo(normalized) ? 'Enter a 2-letter ISO country code.' : null)
  }

  const filterParams = useMemo<TransparencyFilterParams>(() => {
    const filters: TransparencyFilterParams = {}
    if (!fromError && from) filters.from = from
    if (!toError && to) filters.to = to
    if (debouncedPlacementId) filters.placement_id = debouncedPlacementId
    if (debouncedSurface) filters.surface = debouncedSurface
    if (!geoError && debouncedGeo) filters.geo = debouncedGeo
    return filters
  }, [from, to, debouncedPlacementId, debouncedSurface, debouncedGeo, fromError, toError, geoError])

  const params = useMemo(() => ({
    page,
    limit: TRANSPARENCY_AUCTION_PAGE_SIZE,
    ...filterParams,
  }), [page, filterParams])

  useEffect(() => {
    updateParams({
      page: page > 1 ? page : null,
      from: filterParams.from ?? null,
      to: filterParams.to ?? null,
      placement_id: filterParams.placement_id ?? null,
      surface: filterParams.surface ?? null,
      geo: filterParams.geo ?? null,
    })
  }, [page, filterParams, updateParams])

  const initialFiltersRef = useRef(initialFilters)
  const isInitialState = useMemo(() => {
    return page === initialPage && filtersEqual(initialFiltersRef.current, filterParams)
  }, [page, initialPage, filterParams])

  const queryEnabled = !fromError && !toError && !geoError && allowed

  const {
    data: auctionsResponse,
    error,
    isLoading,
    isFetching,
    refetch,
  } = useQuery<AuctionsResponse, Error>({
    queryKey: ['transparency-auctions', params] as const,
    queryFn: ({ signal }) => transparencyApi.list(params, { signal }),
    placeholderData: keepPreviousData,
    enabled: queryEnabled,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: 1,
    retryDelay: 500,
    initialData: allowed && isInitialState && initialData ? initialData : undefined,
  })

  useEffect(() => {
    if (error?.message) {
      setStickyError(error.message)
      return
    }
    if (!isLoading && !isFetching) {
      setStickyError(null)
    }
  }, [error, isLoading, isFetching])

  const auctions: TransparencyAuction[] = auctionsResponse?.data ?? []
  const totalCount = auctionsResponse?.count ?? 0
  const totalPages = Math.max(1, Math.ceil(totalCount / TRANSPARENCY_AUCTION_PAGE_SIZE))
  const disableNext = page >= totalPages
  const showSkeleton = (isLoading && !initialData) || (isFetching && auctions.length === 0)

  const validationAlertMessage = fromError ?? toError ?? geoError ?? null
  const fetchAlertMessage = !validationAlertMessage ? stickyError : null

  const upgradeCta = !allowed && !planLoading

  return (
    <div className="p-6" role="main" aria-labelledby="page-title">
      <div className="flex items-center justify-between mb-4">
        <h1 id="page-title" className="text-xl font-semibold text-gray-900">Transparency — Auctions</h1>
        {canManuallyRefresh ? (
          <button
            type="button"
            onClick={() => refetch()}
            className="text-sm font-medium text-primary-700 hover:text-primary-900"
            aria-label="Refresh auctions"
          >
            Refresh
          </button>
        ) : (
          <span className="text-xs text-gray-400" role="status">
            Manual refresh disabled
          </span>
        )}
      </div>

      {planLoading && (
        <div className="border border-gray-200 rounded-lg bg-white px-4 py-3 mb-4" role="status">
          <p className="text-sm text-gray-700">Checking your plan…</p>
        </div>
      )}

      {upgradeCta && (
        <div className="border border-amber-200 rounded-lg bg-amber-50 px-4 py-3 mb-4" role="alert">
          <p className="text-sm font-semibold text-amber-900">Upgrade to access transparency</p>
          <p className="text-sm text-amber-800 mt-1">
            Transparency auctions, VRA-backed logs, and reconciliation exports are available on Growth and above.
            Starter is limited to basic dashboards. Current plan: {plan || 'unknown'}.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <a href="mailto:billing@apexmediation.ee" className="px-3 py-2 text-sm font-semibold text-white bg-primary-600 rounded-lg hover:bg-primary-700">
              Talk to billing
            </a>
            <a href="/settings/billing/settings" className="px-3 py-2 text-sm font-semibold text-primary-700 border border-primary-200 rounded-lg hover:bg-primary-50">
              Open Billing Settings
            </a>
          </div>
        </div>
      )}

      {planError && !planLoading && (
        <div className="border border-red-200 rounded-lg bg-red-50 px-4 py-3 mb-4" role="alert">
          <p className="text-sm font-semibold text-red-900">Unable to confirm your plan</p>
          <p className="text-xs text-red-800">{planError}</p>
        </div>
      )}

      <form className="grid grid-cols-1 md:grid-cols-5 gap-3 mb-4" aria-label="Auction filters" onSubmit={(e) => e.preventDefault()}>
        <div className="flex flex-col">
          <label htmlFor="fromInput" className="sr-only">From (ISO8601)</label>
          <input id="fromInput" aria-label="From date (ISO8601)" className={`border rounded px-3 py-2 ${fromError ? 'border-red-400' : ''}`} placeholder="From (ISO)" value={from} onChange={(e) => handleFromChange(e.target.value)} />
          {fromError && <span className="text-xs text-red-600 mt-1">{fromError}</span>}
        </div>
        <div className="flex flex-col">
          <label htmlFor="toInput" className="sr-only">To (ISO8601)</label>
          <input id="toInput" aria-label="To date (ISO8601)" className={`border rounded px-3 py-2 ${toError ? 'border-red-400' : ''}`} placeholder="To (ISO)" value={to} onChange={(e) => handleToChange(e.target.value)} />
          {toError && <span className="text-xs text-red-600 mt-1">{toError}</span>}
        </div>
        <div className="flex flex-col">
          <label htmlFor="placementInput" className="sr-only">Placement ID</label>
          <input id="placementInput" aria-label="Placement ID" className="border rounded px-3 py-2" placeholder="Placement ID" value={placementId} onChange={(e) => setPlacementId(e.target.value)} />
        </div>
        <div className="flex flex-col">
          <label htmlFor="surfaceSelect" className="sr-only">Surface</label>
          <select id="surfaceSelect" aria-label="Surface" className="border rounded px-3 py-2" value={surface} onChange={(e) => setSurface(e.target.value)}>
            <option value="">Surface</option>
            <option value="mobile_app">Mobile App</option>
            <option value="web">Web</option>
            <option value="ctv">CTV</option>
          </select>
        </div>
        <div className="flex flex-col">
          <label htmlFor="geoInput" className="sr-only">Geo (country code)</label>
          <input id="geoInput" aria-label="Geo country code" className={`border rounded px-3 py-2 ${geoError ? 'border-red-400' : ''}`} placeholder="Geo (CC)" value={geo} onChange={(e) => handleGeoChange(e.target.value)} maxLength={2} />
          {geoError && <span className="text-xs text-red-600 mt-1">{geoError}</span>}
        </div>
      </form>

      {showSkeleton && <AuctionsTableSkeleton />}
      {!showSkeleton && validationAlertMessage && (
        <div className="border border-amber-200 rounded-lg bg-amber-50 px-4 py-3" role="alert" aria-live="polite">
          <p className="text-sm text-amber-900 font-medium">Check your filters</p>
          <p className="text-xs text-amber-800 mt-1">{validationAlertMessage}</p>
        </div>
      )}

      {!showSkeleton && fetchAlertMessage && (
        <div className="border border-red-200 rounded-lg bg-red-50 px-4 py-3" role="alert" aria-live="assertive">
          <p className="text-sm text-red-900 font-medium">Unable to refresh auctions</p>
          <p className="text-xs text-red-800 mt-1">{fetchAlertMessage}</p>
        </div>
      )}

      {!showSkeleton && !validationAlertMessage && (
        <div className="overflow-x-auto border rounded">
          <table className="min-w-full text-sm" role="table" aria-label="Auctions table">
            <thead className="bg-gray-50 text-gray-600">
              <tr>
                <th scope="col" className="text-left px-3 py-2">Timestamp</th>
                <th scope="col" className="text-left px-3 py-2">Auction ID</th>
                <th scope="col" className="text-left px-3 py-2">Placement</th>
                <th scope="col" className="text-left px-3 py-2">Device</th>
                <th scope="col" className="text-left px-3 py-2">Winner</th>
                <th scope="col" className="text-left px-3 py-2">Verification</th>
              </tr>
            </thead>
            <tbody>
              {auctions.map((row) => (
                <tr className="border-t hover:bg-gray-50" key={row.auction_id}>
                  <td className="px-3 py-2 whitespace-nowrap text-gray-700">
                    {new Date(row.timestamp).toLocaleString()}
                  </td>
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-2">
                      <Link 
                        href={`/transparency/auctions/${row.auction_id}`}
                        className="text-primary-700 hover:underline font-mono text-xs"
                        aria-label={`View auction ${row.auction_id}`}
                      >
                        <span aria-hidden="true">{row.auction_id.slice(0, 8)}...</span>
                        <span className="sr-only">{row.auction_id}</span>
                      </Link>
                      <CopyButton
                        text={row.auction_id}
                        variant="icon"
                        size="sm"
                        label={`Copy auction ID ${row.auction_id}`}
                      />
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-xs text-gray-700" aria-label={`Placement ${row.placement_id}`}>
                        <span aria-hidden="true">{row.placement_id.slice(0, 12)}...</span>
                        <span className="sr-only">{row.placement_id}</span>
                      </span>
                      <CopyButton
                        text={row.placement_id}
                        variant="icon"
                        size="sm"
                        label={`Copy placement ID ${row.placement_id}`}
                      />
                    </div>
                  </td>
                  <td className="px-3 py-2 text-gray-700">
                    <span className="font-medium">{row.device_context.os}</span>
                    <span className="text-gray-400 mx-1">/</span>
                    <span className="font-semibold">{row.device_context.geo}</span>
                  </td>
                  <td className="px-3 py-2">
                    <div className="flex flex-col gap-0.5">
                      <span className="font-semibold text-gray-900">
                        {formatCurrency(row.winner.bid_ecpm, row.winner.currency)}
                      </span>
                      <span className="text-xs text-gray-500">{row.winner.source}</span>
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <VerifyBadge 
                      auctionId={row.auction_id} 
                      hasSigned={!!row.integrity?.signature}
                      compact={true}
                      autoLoad={false}
                    />
                  </td>
                </tr>
              ))}
              {auctions.length === 0 && (
                <tr>
                  <td className="px-3 py-8 text-center text-gray-500" colSpan={6}>
                    <div className="flex flex-col items-center gap-2">
                      <p className="font-medium">No auctions found</p>
                      <p className="text-xs">Try adjusting your filters or date range</p>
                    </div>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}

  {!showSkeleton && !validationAlertMessage && auctions.length > 0 && (
        <div className="flex items-center justify-between mt-4 text-sm text-gray-600">
          <button 
            className="px-4 py-2 border rounded font-medium disabled:opacity-50 disabled:cursor-not-allowed hover:bg-gray-50 transition-colors" 
            onClick={() => setPage((p) => Math.max(1, p - 1))} 
            disabled={page <= 1}
          >
            Previous
          </button>
          <div className="flex flex-col items-center">
            <span className="font-medium">Page {page} of {totalPages}</span>
            <span className="text-xs text-gray-500">{totalCount.toLocaleString()} total auctions</span>
          </div>
          <button 
            className="px-4 py-2 border rounded font-medium hover:bg-gray-50 transition-colors disabled:opacity-50 disabled:cursor-not-allowed" 
            onClick={() => setPage((p) => p + 1)}
            disabled={disableNext}
          >
            Next
          </button>
        </div>
      )}
    </div>
  )}

function AuctionsTableSkeleton() {
  return (
    <div className="overflow-x-auto border rounded">
      <table className="min-w-full text-sm">
        <thead className="bg-gray-50 text-gray-600">
          <tr>
            <th scope="col" className="text-left px-3 py-2">Timestamp</th>
            <th scope="col" className="text-left px-3 py-2">Auction ID</th>
            <th scope="col" className="text-left px-3 py-2">Placement</th>
            <th scope="col" className="text-left px-3 py-2">Device</th>
            <th scope="col" className="text-left px-3 py-2">Winner</th>
            <th scope="col" className="text-left px-3 py-2">Verification</th>
          </tr>
        </thead>
        <tbody>
          {AUCTIONS_TABLE_SKELETON_KEYS.map((key) => (
            <tr key={key} className="border-t">
              <td className="px-3 py-2">
                <Skeleton width="w-32" height="h-4" />
              </td>
              <td className="px-3 py-2">
                <Skeleton width="w-20" height="h-4" />
              </td>
              <td className="px-3 py-2">
                <Skeleton width="w-24" height="h-4" />
              </td>
              <td className="px-3 py-2">
                <Skeleton width="w-16" height="h-4" />
              </td>
              <td className="px-3 py-2">
                <Skeleton width="w-20" height="h-4" />
              </td>
              <td className="px-3 py-2">
                <Skeleton width="w-16" height="h-6" />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
