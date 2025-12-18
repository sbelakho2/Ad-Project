 'use client'

import Link from 'next/link'
import { useQuery } from '@tanstack/react-query'
import { getCurrentUsage, type UsageData } from '@/lib/billing'
import { getPlatformTierDisplay } from '@/lib/platformTiers'
import { AlertCircle, TrendingUp, Database, Zap, HardDrive } from 'lucide-react'
import { formatNumber, formatCurrency, formatDate } from '@/lib/utils'
import { Button } from '@/ui-v2/components/Button'
import { Select } from '@/ui-v2/components/Select'
import { useMemo, useState, useCallback, type ChangeEvent } from 'react'

const escapeCsv = (value: string | number | null | undefined) => {
  if (value === null || typeof value === 'undefined') return ''
  const stringValue = String(value)
  if (/[",\n]/.test(stringValue)) {
    return `"${stringValue.replace(/"/g, '""')}"`
  }
  return stringValue
}

type RangeOption = '7d' | '30d' | '90d' | 'all'

export default function BillingUsagePage() {
  const [range, setRange] = useState<RangeOption>('30d')
  const [product, setProduct] = useState<string>('all')
  const [page, setPage] = useState<number>(1)
  const PAGE_SIZE = 25
  const { data: usage, isLoading: loading, error, refetch } = useQuery<UsageData>({
    queryKey: ['billing', 'usage', 'current', range],
    queryFn: ({ signal }) => getCurrentUsage({ signal }),
    staleTime: 60 * 1000,
  })

  // Derive products and filtered/paginated rows client-side for now
  const productOptions = useMemo(() => {
    const set = new Set<string>()
    ;(usage?.items || []).forEach((it) => {
      if (it.product) set.add(it.product)
    })
    return Array.from(set).sort()
  }, [usage?.items])

  const filteredItems = useMemo(() => {
    const all = usage?.items || []
    const byProduct = product === 'all' ? all : all.filter((it) => (it.product ?? '—') === product)
    // Reset page when filter changes (handled below in effect-like memo by referencing deps)
    return byProduct
  }, [usage?.items, product])

  const totalPages = Math.max(1, Math.ceil(filteredItems.length / PAGE_SIZE))
  const currentPage = Math.min(page, totalPages)
  const pagedItems = useMemo(() => {
    const start = (currentPage - 1) * PAGE_SIZE
    return filteredItems.slice(start, start + PAGE_SIZE)
  }, [filteredItems, currentPage])

  const csvRows = useMemo(() => {
    if (!usage) return [] as string[]
    const header = ['Date', 'Product', 'Quantity', 'Unit Price', 'Subtotal']
    const rows = (filteredItems || []).map((it) => [
      formatDate(it.date),
      it.product ?? '—',
      String(it.quantity ?? 0),
      formatCurrency(it.unit_price ?? 0),
      formatCurrency((it.quantity ?? 0) * (it.unit_price ?? 0)),
    ])
    return [header, ...rows].map((cols) => cols.map(escapeCsv).join(','))
  }, [usage, filteredItems])

  const handleRangeChange = useCallback((event: ChangeEvent<HTMLSelectElement>) => {
    setRange(event.target.value as RangeOption)
    setPage(1)
  }, [])

  const handleProductChange = useCallback((event: ChangeEvent<HTMLSelectElement>) => {
    setProduct(event.target.value)
    setPage(1)
  }, [])

  const handleRefresh = useCallback(() => {
    void refetch()
  }, [refetch])

  const handleExportCsv = useCallback(() => {
    if (!csvRows.length) return
    const blob = new Blob([csvRows.join('\n')], { type: 'text/csv;charset=utf-8;' })
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `usage-${range}-${new Date().toISOString().slice(0, 10)}.csv`
    anchor.click()
    URL.revokeObjectURL(url)
  }, [csvRows, range])

  if (loading) {
    return (
      <div className="p-8">
        <div className="max-w-6xl mx-auto">
          <div className="animate-pulse space-y-6">
            <div className="h-8 w-48 bg-gray-200 rounded"></div>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
              {[1, 2, 3].map((i) => (
                <div key={i} className="h-40 bg-gray-200 rounded-lg"></div>
              ))}
            </div>
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <div className="max-w-6xl mx-auto">
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 flex items-start gap-3">
            <AlertCircle className="h-5 w-5 text-red-600 mt-0.5" />
            <div>
              <h3 className="font-semibold text-red-900">Error Loading Usage</h3>
              <p className="text-sm text-red-700 mt-1">{(error as Error).message}</p>
              <button
                onClick={handleRefresh}
                className="mt-3 text-sm font-medium text-red-700 hover:text-red-800"
              >
                Try Again
              </button>
            </div>
          </div>
        </div>
      </div>
    )
  }

  if (!usage) {
    return (
      <div className="min-h-screen bg-gray-50 px-6 py-12">
        <div className="mx-auto max-w-4xl text-center">
          <div className="rounded-xl border border-dashed border-gray-200 bg-white/70 px-6 py-10 shadow-sm">
            <p className="text-sm font-semibold text-gray-900">No usage for this period yet.</p>
            <p className="mt-2 text-sm text-gray-600">Adjust filters or refresh to check again.</p>
            <div className="mt-4 inline-flex items-center gap-3">
              <Select aria-label="Date range" value={range} onChange={handleRangeChange}>
                <option value="7d">Last 7 days</option>
                <option value="30d">Last 30 days</option>
                <option value="90d">Last 90 days</option>
                <option value="all">All time</option>
              </Select>
              <Button variant="secondary" onClick={handleRefresh}>Refresh</Button>
            </div>
          </div>
        </div>
      </div>
    )
  }

  const subscriptionTier = getPlatformTierDisplay(usage.subscription.plan_type)

  const getUsageState = (used: number, limit: number) => {
    const safeLimit = Math.max(limit, 1)
    const percentage = Math.min((used / safeLimit) * 100, 999)
    if (percentage > 100) {
      return {
        percentage,
        badgeText: 'Over limit',
        badgeClass: 'text-red-700 bg-red-50 border-red-200',
        barClass: 'bg-red-500',
        textClass: 'text-red-700',
      }
    }
    if (percentage >= 70) {
      return {
        percentage,
        badgeText: 'Approaching limit',
        badgeClass: 'text-amber-700 bg-amber-50 border-amber-200',
        barClass: 'bg-amber-500',
        textClass: 'text-amber-700',
      }
    }
    return {
      percentage,
      badgeText: 'Within limit',
      badgeClass: 'text-green-700 bg-green-50 border-green-200',
      barClass: 'bg-green-500',
      textClass: 'text-green-700',
    }
  }

  const usageCards = [
    {
      key: 'impressions',
      label: 'Impressions',
      caption: 'Ad views this period',
      icon: TrendingUp,
      iconBg: 'bg-blue-100',
      iconColor: 'text-blue-600',
      used: usage.current_period.impressions,
      limit: usage.subscription.included_impressions,
      overageAmount: usage.overages.impressions.amount,
      overageCost: usage.overages.impressions.cost,
      formattedUsed: formatNumber(usage.current_period.impressions),
      formattedLimit: formatNumber(usage.subscription.included_impressions),
      unitLabel: 'impressions',
    },
    {
      key: 'apiCalls',
      label: 'API Calls',
      caption: 'Requests this period',
      icon: Zap,
      iconBg: 'bg-violet-100',
      iconColor: 'text-violet-600',
      used: usage.current_period.api_calls,
      limit: usage.subscription.included_api_calls,
      overageAmount: usage.overages.api_calls.amount,
      overageCost: usage.overages.api_calls.cost,
      formattedUsed: formatNumber(usage.current_period.api_calls),
      formattedLimit: formatNumber(usage.subscription.included_api_calls),
      unitLabel: 'calls',
    },
    {
      key: 'dataTransfer',
      label: 'Data Transfer',
      caption: 'Bandwidth this period',
      icon: HardDrive,
      iconBg: 'bg-emerald-100',
      iconColor: 'text-emerald-600',
      used: usage.current_period.data_transfer_gb,
      limit: usage.subscription.included_data_transfer_gb,
      overageAmount: usage.overages.data_transfer.amount,
      overageCost: usage.overages.data_transfer.cost,
      formattedUsed: `${usage.current_period.data_transfer_gb.toFixed(2)} GB`,
      formattedLimit: `${formatNumber(usage.subscription.included_data_transfer_gb)} GB`,
      unitLabel: 'GB',
    },
  ] as const

  return (
    <div className="min-h-screen bg-gray-50">
      <div className="mx-auto max-w-6xl px-6 py-10 space-y-10">
        {/* Header */}
        <div className="flex flex-col gap-6">
          <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
            <div className="space-y-2">
              <h1 className="text-2xl font-semibold tracking-tight text-gray-900">Billing</h1>
              <p className="text-sm text-gray-600">Usage & invoices for your account.</p>
              <p className="text-sm text-gray-500">
                Current period: {formatDate(usage.current_period.start)} – {formatDate(usage.current_period.end)} ·{' '}
                <span className="font-semibold text-gray-800">{subscriptionTier.label} plan</span>
              </p>
              <p className="text-xs font-medium text-gray-500">
                2.0% platform fee (1.25% with annual prepay) · Includes RBAC, BI exports and dedicated integration support.
              </p>
            </div>
            <div className="flex flex-col items-start gap-2 md:items-end">
              <div className="inline-flex items-center gap-2 rounded-full border border-gray-200 bg-white px-4 py-2 shadow-sm">
                <span className="text-sm font-medium text-gray-900">{subscriptionTier.label} plan</span>
                <Link className="text-sm font-medium text-primary-700 hover:text-primary-800" href="/settings/billing/settings">
                  Change plan
                </Link>
              </div>
              <p className="text-xs text-gray-500">2.0% platform fee · 1.25% with annual prepay</p>
            </div>
          </div>

          {/* Filters */}
          <div className="flex flex-col gap-3 rounded-xl border border-gray-200 bg-white/80 px-4 py-3 shadow-sm md:flex-row md:items-center md:justify-between">
            <div className="flex flex-wrap items-center gap-3 text-sm text-gray-700">
              <label className="text-sm text-gray-600" htmlFor="usage-range">Date range</label>
              <Select id="usage-range" aria-label="Date range" value={range} onChange={handleRangeChange}>
                <option value="7d">Last 7 days</option>
                <option value="30d">Last 30 days</option>
                <option value="90d">Last 90 days</option>
                <option value="all">All time</option>
              </Select>
              <label className="text-sm text-gray-600" htmlFor="usage-product">Product</label>
              <Select id="usage-product" aria-label="Product filter" value={product} onChange={handleProductChange}>
                <option value="all">All products</option>
                {productOptions.map((p) => (
                  <option key={p} value={p}>{p}</option>
                ))}
              </Select>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="ghost" size="sm" onClick={handleRefresh} aria-label="Refresh usage">
                Refresh
              </Button>
              <Button variant="secondary" size="sm" onClick={handleExportCsv} aria-label="Export CSV">
                Export CSV
              </Button>
            </div>
          </div>
        </div>

        {/* Usage Metrics */}
        <section aria-labelledby="usage-overview-heading" className="space-y-4">
          <div className="flex items-start justify-between gap-3">
            <div className="space-y-1">
              <h2 id="usage-overview-heading" className="text-lg font-semibold text-gray-900">
                Usage Overview
              </h2>
              <p className="text-sm text-gray-600">See how your usage compares to your plan limits.</p>
            </div>
            <span className="hidden text-xs text-gray-500 md:inline">Updated hourly</span>
          </div>
          <div className="grid grid-cols-1 gap-6 md:grid-cols-3">
            {usageCards.map((card) => {
              const Icon = card.icon
              const state = getUsageState(card.used, card.limit)
              return (
                <div
                  key={card.key}
                  className="flex flex-col gap-4 rounded-xl border border-gray-200 bg-white p-5 shadow-sm"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex items-center gap-3">
                      <div className={`flex h-10 w-10 items-center justify-center rounded-xl ${card.iconBg}`}>
                        <Icon className={`h-5 w-5 ${card.iconColor}`} />
                      </div>
                      <div className="leading-tight">
                        <p className="text-sm font-semibold text-gray-900">{card.label}</p>
                        <p className="text-xs text-gray-600">{card.caption}</p>
                      </div>
                    </div>
                    <span
                      className={`inline-flex items-center rounded-full border px-3 py-1 text-xs font-medium ${state.badgeClass}`}
                    >
                      {state.badgeText}
                    </span>
                  </div>

                  <div className="space-y-2">
                    <div className="flex items-baseline gap-2">
                      <span className="text-2xl font-semibold text-gray-900">{card.formattedUsed}</span>
                      <span className="text-sm text-gray-600">of {card.formattedLimit} included</span>
                    </div>
                    <progress
                      aria-label={`${card.label} usage progress`}
                      value={Math.min(state.percentage, 100)}
                      max={100}
                      className="sr-only"
                    >
                      {state.percentage.toFixed(1)}%
                    </progress>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-gray-200">
                      <div
                        className={`h-full ${state.barClass} transition-all duration-300`}
                        style={{ width: `${Math.min(state.percentage, 100)}%` }}
                      />
                    </div>
                    <p className={`text-sm font-medium ${state.textClass}`}>
                      {state.percentage.toFixed(1)}% of plan used
                    </p>
                  </div>

                  <div className="border-t border-gray-100 pt-3 text-sm text-gray-700">
                    {card.overageAmount > 0 ? (
                      <p>
                        Overage so far: {formatNumber(card.overageAmount)} {card.unitLabel} ({formatCurrency(card.overageCost)})
                      </p>
                    ) : (
                      <p className="text-gray-500">No overages this period.</p>
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        </section>

        {/* Total Overage Summary */}
        {usage.overages.total_overage_cost > 0 && (
          <section aria-labelledby="usage-overage-heading" className="space-y-3">
            <div className="flex items-start justify-between gap-3">
              <div className="space-y-1">
                <h2 id="usage-overage-heading" className="text-lg font-semibold text-gray-900">
                  Overage Summary
                </h2>
                <p className="text-sm text-gray-600">This period you’re over your included limits by:</p>
              </div>
              <Button variant="secondary" size="sm">View invoice estimate</Button>
            </div>
            <div className="rounded-xl border border-amber-200 bg-amber-50 p-5">
              <div className="flex items-start gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-amber-100 text-amber-700">
                  <AlertCircle className="h-5 w-5" aria-hidden />
                </div>
                <div className="flex-1 space-y-2 text-sm text-gray-800">
                  <div className="flex items-center justify-between gap-4">
                    <span className="text-gray-700">Impressions</span>
                    <span className="font-semibold text-gray-900">
                      {formatNumber(usage.overages.impressions.amount)} ({formatCurrency(usage.overages.impressions.cost)})
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-4">
                    <span className="text-gray-700">API calls</span>
                    <span className="font-semibold text-gray-900">
                      {formatNumber(usage.overages.api_calls.amount)} ({formatCurrency(usage.overages.api_calls.cost)})
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-4">
                    <span className="text-gray-700">Data transfer</span>
                    <span className="font-semibold text-gray-900">
                      {usage.overages.data_transfer.amount.toFixed(2)} GB ({formatCurrency(usage.overages.data_transfer.cost)})
                    </span>
                  </div>
                  <div className="pt-2 text-sm font-semibold text-amber-800">
                    Total overage: {formatCurrency(usage.overages.total_overage_cost)}
                  </div>
                </div>
              </div>
            </div>
          </section>
        )}

        {/* Info Box */}
        <section aria-labelledby="usage-insights-heading" className="space-y-4">
          <h2 id="usage-insights-heading" className="text-lg font-semibold text-gray-900">
            Usage Notes
          </h2>
          <div className="rounded-xl border border-blue-100 bg-blue-50 p-5">
            <div className="flex items-start gap-4">
              <Database className="h-6 w-6 text-blue-600 flex-shrink-0 mt-0.5" />
              <div className="space-y-1 text-sm text-gray-700">
                <p>Usage updates hourly. Overages bill at the end of each period.</p>
                <p>Export CSV for detailed rows or switch products to narrow the view.</p>
              </div>
            </div>
          </div>
        </section>

        {/* Usage Table (paged) — lightweight client pagination until server paging is wired) */}
        <section className="mt-8 space-y-3" aria-labelledby="usage-table-heading">
          <div className="flex items-center justify-between">
            <div className="space-y-1">
              <h2 id="usage-table-heading" className="text-lg font-semibold text-gray-900">Usage Details</h2>
              <p className="text-sm text-gray-600">Line items for this period.</p>
            </div>
          </div>
          <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-gray-200">
              <thead className="bg-gray-50">
                <tr>
                  <th scope="col" className="px-4 py-2 text-left text-xs font-medium text-gray-500">Date</th>
                  <th scope="col" className="px-4 py-2 text-left text-xs font-medium text-gray-500">Product</th>
                  <th scope="col" className="px-4 py-2 text-right text-xs font-medium text-gray-500">Quantity</th>
                  <th scope="col" className="px-4 py-2 text-right text-xs font-medium text-gray-500">Unit Price</th>
                  <th scope="col" className="px-4 py-2 text-right text-xs font-medium text-gray-500">Subtotal</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100 bg-white">
                {pagedItems.length === 0 ? (
                  <tr>
                    <td colSpan={5} className="px-4 py-8 text-center text-sm text-gray-500">
                      No usage matches the current filters.
                    </td>
                  </tr>
                ) : (
                  pagedItems.map((it) => {
                    const rowKey = [it.date, it.product ?? '—', it.quantity ?? 0, it.unit_price ?? 0].join('-')
                    return (
                    <tr key={rowKey}>
                      <td className="px-4 py-2 text-sm text-gray-700">{formatDate(it.date)}</td>
                      <td className="px-4 py-2 text-sm text-gray-700">{it.product ?? '—'}</td>
                      <td className="px-4 py-2 text-right text-sm text-gray-700">{formatNumber(it.quantity ?? 0)}</td>
                      <td className="px-4 py-2 text-right text-sm text-gray-700">{formatCurrency(it.unit_price ?? 0)}</td>
                      <td className="px-4 py-2 text-right text-sm text-gray-700">{formatCurrency((it.quantity ?? 0) * (it.unit_price ?? 0))}</td>
                    </tr>
                  )})
                )}
              </tbody>
            </table>
          </div>
          {/* Pagination controls */}
          <div className="mt-3 flex items-center justify-between">
            <p className="text-sm text-gray-600">
              Showing {pagedItems.length === 0 ? 0 : (currentPage - 1) * PAGE_SIZE + 1}–
              {Math.min(currentPage * PAGE_SIZE, filteredItems.length)} of {filteredItems.length}
            </p>
            <div className="flex items-center gap-2">
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                disabled={currentPage <= 1}
                aria-label="Previous page"
              >
                Prev
              </Button>
              <span className="text-sm text-gray-700">
                Page {currentPage} / {totalPages}
              </span>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                disabled={currentPage >= totalPages}
                aria-label="Next page"
              >
                Next
              </Button>
            </div>
          </div>
        </section>
      </div>
    </div>
  )
}
