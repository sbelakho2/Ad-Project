'use client'

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { usePathname, useRouter, useSearchParams } from 'next/navigation'
import { useQuery } from '@tanstack/react-query'
import { payoutApi } from '@/lib/api'
import {
  DollarSign,
  Clock,
  Download,
  Calendar,
  CreditCard,
} from 'lucide-react'
import { formatCurrency, formatDate, getLocale } from '@/lib/utils'
import type { PaginatedResponse, PayoutHistory } from '@/types'
import { useSession } from '@/lib/useSession'
import type { Role } from '@/lib/rbac'
import {
  PAYOUT_CSV_HEADERS,
  PAYOUT_METHOD_LABELS,
  PAYOUT_STATUS_META,
  PAYOUTS_PAGE_SIZE,
} from '@/constants/payouts'
import { Section, Container, PageHeader } from '@/components/ui'

const PAYOUT_ALLOWED_ROLES: Role[] = ['admin', 'publisher']

const parsePositivePage = (value: string | null, fallback = 1) => {
  const parsed = Number(value)
  if (Number.isFinite(parsed) && parsed >= 1) {
    return Math.floor(parsed)
  }
  return fallback
}

const escapeCsvValue = (value: string | number | null | undefined) => {
  if (value === null || typeof value === 'undefined') return ''
  const stringValue = String(value)
  if (/[",\n]/.test(stringValue)) {
    return `"${stringValue.replace(/"/g, '""')}"`
  }
  return stringValue
}

export default function PayoutsPage() {
  const router = useRouter()
  const pathname = usePathname()
  const searchParams = useSearchParams()
  const searchParamsString = useMemo(() => searchParams.toString(), [searchParams])
  const { user, isLoading: sessionLoading } = useSession()
  const [page, setPage] = useState(() => parsePositivePage(searchParams.get('page')))
  const [exportError, setExportError] = useState<string | null>(null)
  const pageSize = PAYOUTS_PAGE_SIZE
  const resolvedLocale = useMemo(() => user?.locale ?? getLocale(), [user?.locale])
  const publisherId = user?.publisherId

  useEffect(() => {
    const paramPage = parsePositivePage(searchParams.get('page'))
    setPage((prev) => (prev === paramPage ? prev : paramPage))
  }, [searchParams])

  const csvDateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(resolvedLocale, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
      }),
    [resolvedLocale]
  )

  const currencyFormatterCache = useRef(new Map<string, Intl.NumberFormat>())

  useEffect(() => {
    currencyFormatterCache.current.clear()
  }, [resolvedLocale])

  const formatCsvDate = useCallback(
    (value?: string | null) => {
      if (!value) return '—'
      const parsed = new Date(value)
      if (Number.isNaN(parsed.getTime())) {
        return '—'
      }
      return csvDateFormatter.format(parsed)
    },
    [csvDateFormatter]
  )

  const formatCsvCurrency = useCallback(
    (amount: number, currency: string) => {
      const code = currency || 'USD'
      let formatter = currencyFormatterCache.current.get(code)
      if (!formatter) {
        formatter = new Intl.NumberFormat(resolvedLocale, { style: 'currency', currency: code })
        currencyFormatterCache.current.set(code, formatter)
      }
      return formatter.format(amount)
    },
    [resolvedLocale]
  )

  const userRole = (user?.role ?? 'publisher') as Role
  const canView = !!user && PAYOUT_ALLOWED_ROLES.includes(userRole)
  const canFetchPayouts = canView && Boolean(publisherId)

  const syncPageState = useCallback(
    (nextPage: number) => {
      setPage(nextPage)
      const params = new URLSearchParams(searchParamsString)
      if (nextPage > 1) {
        params.set('page', String(nextPage))
      } else {
        params.delete('page')
      }
      const nextSearch = params.toString()
      router.replace(nextSearch ? `${pathname}?${nextSearch}` : pathname, { scroll: false })
    },
    [pathname, router, searchParamsString]
  )

  useEffect(() => {
    if (!sessionLoading && !user) {
      router.replace('/login')
    }
  }, [sessionLoading, user, router])

  useEffect(() => {
    if (!sessionLoading && user && !canView) {
      router.replace('/403')
    }
  }, [sessionLoading, user, canView, router])

  const {
    data: historyData,
    isLoading: loadingHistory,
  } = useQuery<PaginatedResponse<PayoutHistory>>({
    queryKey: ['payout-history', page, publisherId],
    enabled: canFetchPayouts,
    queryFn: async ({ signal }) => {
      return payoutApi.getHistory({ page, pageSize, publisherId, signal })
    },
  })

  const {
    data: upcoming,
    isLoading: loadingUpcoming,
  } = useQuery<PayoutHistory | null>({
    queryKey: ['payout-upcoming', publisherId],
    enabled: canFetchPayouts,
    queryFn: async ({ signal }) => {
      return payoutApi.getUpcoming({ publisherId, signal })
    },
  })

  const history = useMemo<PayoutHistory[]>(() => historyData?.data ?? [], [historyData])
  const hasMore = historyData?.hasMore || false
  const totalPages = useMemo(() => {
    if (historyData?.total) {
      const size = historyData.pageSize || pageSize
      return Math.max(1, Math.ceil(historyData.total / size))
    }
    return hasMore ? page + 1 : historyData ? page : null
  }, [hasMore, historyData, page, pageSize])
  const canGoNext = totalPages ? page < totalPages : hasMore
  const canGoPrev = page > 1

  useEffect(() => {
    if (totalPages && page > totalPages) {
      syncPageState(totalPages)
    }
  }, [page, totalPages, syncPageState])

  const goToPage = useCallback(
    (targetPage: number) => {
      const normalized = Math.max(1, totalPages ? Math.min(totalPages, targetPage) : targetPage)
      syncPageState(normalized)
    },
    [syncPageState, totalPages]
  )

  const handleNextPage = useCallback(() => {
    if (canGoNext) {
      goToPage(page + 1)
    }
  }, [canGoNext, goToPage, page])

  const handlePrevPage = useCallback(() => {
    if (canGoPrev) {
      goToPage(page - 1)
    }
  }, [canGoPrev, goToPage, page])

  const handleExportCSV = useCallback(() => {
    if ((!history || history.length === 0) && !upcoming) {
      setExportError('No payout data available to export yet.')
      return
    }

    try {
      const rows: string[][] = [PAYOUT_CSV_HEADERS.map((header) => header.label)]

      history.forEach((payout) => {
        const statusMeta = PAYOUT_STATUS_META[payout.status]
        const methodLabel = PAYOUT_METHOD_LABELS[payout.method] ?? payout.method
        rows.push([
          payout.id,
          formatCsvDate(payout.scheduledFor),
          formatCsvDate(payout.processedAt),
          formatCsvCurrency(payout.amount, payout.currency),
          statusMeta.label,
          methodLabel,
        ])
      })

      if (upcoming) {
        const upcomingStatus = PAYOUT_STATUS_META[upcoming.status]
        const methodLabel = PAYOUT_METHOD_LABELS[upcoming.method] ?? upcoming.method
        rows.push([])
        rows.push(['Upcoming Payout'])
        rows.push([
          upcoming.id,
          formatCsvDate(upcoming.scheduledFor),
          '—',
          formatCsvCurrency(upcoming.amount, upcoming.currency),
          upcomingStatus.label,
          methodLabel,
        ])
      }

      const csvContent = rows.map((row) => row.map(escapeCsvValue).join(',')).join('\r\n')
      const blob = new Blob([csvContent], { type: 'text/csv;charset=utf-8;' })
      const url = URL.createObjectURL(blob)

      const link = document.createElement('a')
      link.href = url
      const filenameDate = new Date().toISOString().split('T')[0]
      link.download = `payouts-${publisherId ?? 'export'}-${filenameDate}.csv`
      document.body.appendChild(link)
      link.click()
      document.body.removeChild(link)
      URL.revokeObjectURL(url)
      setExportError(null)
    } catch (error) {
      console.error('Unable to export payouts', error)
      setExportError('Unable to export payouts right now. Please try again.')
    }
  }, [formatCsvCurrency, formatCsvDate, history, publisherId, upcoming])

  if (sessionLoading) {
    return (
      <div className="min-h-screen bg-[#f9fafb] px-6 py-12" role="status" aria-live="polite">
        <div className="max-w-4xl mx-auto">
          <div className="animate-pulse space-y-4">
            <div className="h-6 w-40 rounded bg-gray-200" />
            <div className="h-32 rounded bg-gray-200" />
            <p className="text-sm text-gray-500">Loading payouts…</p>
          </div>
        </div>
      </div>
    )
  }

  if (!canView) {
    return (
      <div className="min-h-screen bg-[#f9fafb] px-6 py-12">
        <div className="max-w-4xl mx-auto">
          <div className="rounded-xl border border-gray-200 bg-white p-6" role="alert">
            <h2 className="text-lg font-semibold text-gray-900">Access restricted</h2>
            <p className="mt-2 text-sm text-gray-600">
              You don&apos;t have permission to view payouts. Contact an administrator if this seems wrong.
            </p>
          </div>
        </div>
      </div>
    )
  }

  if (!publisherId) {
    return (
      <div className="min-h-screen bg-[#f9fafb] px-6 py-12">
        <div className="max-w-4xl mx-auto">
          <div className="rounded-xl border border-gray-200 bg-white p-6" role="alert">
            <h2 className="text-lg font-semibold text-gray-900">Missing publisher context</h2>
            <p className="mt-2 text-sm text-gray-600">
              We couldn&apos;t determine which publisher to load payouts for. Refresh or contact support if this continues.
            </p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="min-h-screen bg-[#f9fafb]">
      <PageHeader
        kicker="Finance"
        title="Payouts"
        subtitle="Track payment history and upcoming scheduled payouts."
        actions={
          <div className="flex flex-col items-end">
            <button type="button" onClick={handleExportCSV} className="btn btn-outline flex items-center gap-2">
              <Download className="h-4 w-4" aria-hidden={true} />
              Export CSV
            </button>
            {exportError && (
              <p className="text-sm text-red-600 mt-2" role="alert">
                {exportError}
              </p>
            )}
          </div>
        }
      />

      <Section>
        <Container className="space-y-6">
          {/* Upcoming payout */}
          {loadingUpcoming ? (
            <div className="card animate-pulse">
            <div className="h-6 bg-gray-200 rounded w-1/3 mb-4" />
            <div className="h-20 bg-gray-200 rounded" />
          </div>
        ) : upcoming ? (
          <div className="card bg-gradient-to-br from-primary-50 to-white border-primary-200">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h2 className="text-lg font-semibold text-gray-900 flex items-center gap-2">
                  <Calendar className="h-5 w-5 text-primary-600" aria-hidden={true} />
                  Next Scheduled Payout
                </h2>
                <p className="text-sm text-gray-600 mt-1">
                  Funds will be sent on {formatDate(upcoming.scheduledFor)}
                </p>
              </div>
              {(() => {
                const statusMeta = PAYOUT_STATUS_META[upcoming.status]
                const Icon = statusMeta.icon
                return (
                  <span
                    className={`inline-flex items-center gap-2 px-3 py-1 rounded-full text-sm font-medium border ${statusMeta.subtleClass}`}
                  >
                    <Icon className="h-4 w-4" aria-hidden={true} />
                    {statusMeta.label}
                  </span>
                )
              })()}
            </div>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-6 p-4 bg-white rounded-lg border">
              <div>
                <p className="text-sm text-gray-600">Amount</p>
                <p className="text-2xl font-bold text-gray-900 mt-1">
                  {formatCurrency(upcoming.amount, upcoming.currency)}
                </p>
              </div>
              <div>
                <p className="text-sm text-gray-600">Payment Method</p>
                <div className="flex items-center gap-2 mt-1">
                  <CreditCard className="h-5 w-5 text-gray-400" aria-hidden={true} />
                  <p className="text-lg font-semibold text-gray-900">
                    {PAYOUT_METHOD_LABELS[upcoming.method] ?? upcoming.method}
                  </p>
                </div>
              </div>
              <div>
                <p className="text-sm text-gray-600">Expected Date</p>
                <p className="text-lg font-semibold text-gray-900 mt-1">
                  {formatDate(upcoming.scheduledFor)}
                </p>
              </div>
            </div>
          </div>
        ) : (
          <div className="card text-center py-12">
            <Calendar className="h-12 w-12 text-gray-400 mx-auto mb-3" aria-hidden={true} />
            <p className="text-sm text-gray-600">No upcoming payouts scheduled</p>
            <p className="text-xs text-gray-500 mt-1">Payments are issued when you reach the minimum threshold</p>
          </div>
        )}

        {/* Payment history */}
        <div className="card">
          <h2 className="text-lg font-semibold text-gray-900 mb-4">Payment History</h2>

          {loadingHistory ? (
            <div className="space-y-3">
              {[1, 2, 3, 4, 5].map((i) => (
                <div key={i} className="p-4 border rounded-lg animate-pulse">
                  <div className="flex items-center gap-4">
                    <div className="h-10 w-10 bg-gray-200 rounded-full" />
                    <div className="flex-1 space-y-2">
                      <div className="h-4 bg-gray-200 rounded w-1/3" />
                      <div className="h-3 bg-gray-200 rounded w-1/2" />
                    </div>
                  </div>
                </div>
              ))}
            </div>
          ) : history.length === 0 ? (
            <div className="text-center py-12">
              <DollarSign className="h-12 w-12 text-gray-400 mx-auto mb-3" aria-hidden={true} />
              <p className="text-sm text-gray-600">No payment history yet</p>
              <p className="text-xs text-gray-500 mt-1">Your first payout will appear here once processed</p>
            </div>
          ) : (
            <>
              <div className="overflow-x-auto">
                <table className="w-full">
                  <thead className="bg-gray-50 border-y">
                    <tr>
                      <th scope="col" className="px-4 py-3 text-left text-xs font-medium text-gray-600 uppercase tracking-wider">
                        Date
                      </th>
                      <th scope="col" className="px-4 py-3 text-left text-xs font-medium text-gray-600 uppercase tracking-wider">
                        Amount
                      </th>
                      <th scope="col" className="px-4 py-3 text-left text-xs font-medium text-gray-600 uppercase tracking-wider">
                        Method
                      </th>
                      <th scope="col" className="px-4 py-3 text-left text-xs font-medium text-gray-600 uppercase tracking-wider">
                        Status
                      </th>
                      <th scope="col" className="px-4 py-3 text-left text-xs font-medium text-gray-600 uppercase tracking-wider">
                        Completed
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y">
                    {history.map((payout) => {
                      const statusMeta = PAYOUT_STATUS_META[payout.status]
                      const StatusIcon = statusMeta.icon
                      return (
                        <tr key={payout.id} className="hover:bg-gray-50">
                          <td className="px-4 py-4 text-sm text-gray-900">
                            {formatDate(payout.scheduledFor)}
                          </td>
                          <td className="px-4 py-4 text-sm font-semibold text-gray-900">
                            {formatCurrency(payout.amount, payout.currency)}
                          </td>
                          <td className="px-4 py-4 text-sm text-gray-600">
                            <div className="flex items-center gap-2">
                              <CreditCard className="h-4 w-4 text-gray-400" aria-hidden={true} />
                              {PAYOUT_METHOD_LABELS[payout.method] ?? payout.method}
                            </div>
                          </td>
                          <td className="px-4 py-4">
                            <span
                              className={`inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium border ${statusMeta.subtleClass}`}
                            >
                              <StatusIcon className="h-3 w-3" aria-hidden={true} />
                              {statusMeta.label}
                            </span>
                          </td>
                          <td className="px-4 py-4 text-sm text-gray-600">
                            {payout.processedAt ? formatDate(payout.processedAt) : '—'}
                          </td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>

              {/* Pagination */}
              {(canGoPrev || canGoNext) && (
                <div className="flex items-center justify-between mt-6 pt-6 border-t">
                  <button
                    onClick={handlePrevPage}
                    disabled={!canGoPrev}
                    className="btn btn-outline disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    Previous
                  </button>
                  <span className="text-sm text-gray-600">
                    Page {page}
                    {totalPages ? ` of ${totalPages}` : ''}
                  </span>
                  <button
                    onClick={handleNextPage}
                    disabled={!canGoNext}
                    className="btn btn-outline disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    Next
                  </button>
                </div>
              )}
            </>
          )}
        </div>
        </Container>
      </Section>
    </div>
  )
}
