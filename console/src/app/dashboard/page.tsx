'use client'

import Link from 'next/link'
import { useSession } from 'next-auth/react'
import DashboardGate from './DashboardGate'
import { useQuery } from '@tanstack/react-query'
import { useCallback, useMemo, useState, type ChangeEvent } from 'react'
import { revenueApi } from '@/lib/api'
import { formatCurrency, formatNumber, formatPercentage } from '@/lib/utils'
import { MetricCard, MetricCardSkeleton } from '@/components/dashboard/MetricCard'
import { DashboardCharts } from '@/components/dashboard/DashboardCharts'
import { FraudWidget } from '@/components/dashboard/FraudWidget'
import { PayoutWidget } from '@/components/dashboard/PayoutWidget'
import {
  CalendarRange,
  DollarSign,
  Eye,
  MousePointer,
  TrendingUp,
  Download,
  PlusCircle,
  Settings2,
  BarChart3,
  CreditCard,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

export default function DashboardPage() {
  const { data: session } = useSession()
  const defaultStart = useMemo(
    () => new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString().split('T')[0],
    []
  )
  const today = useMemo(() => new Date().toISOString().split('T')[0], [])

  const [dateRange, setDateRange] = useState({
    startDate: defaultStart,
    endDate: today,
  })
  const [exportError, setExportError] = useState<string | null>(null)

  const {
    data: revenueSummary,
    isLoading: loadingRevenue,
    isError: revenueError,
    error: revenueSummaryError,
    refetch: refetchRevenueSummary,
  } = useQuery({
    queryKey: ['revenue-summary', dateRange.startDate, dateRange.endDate],
    queryFn: async ({ signal }) => {
      const { data } = await revenueApi.getSummary({
        startDate: dateRange.startDate,
        endDate: dateRange.endDate,
        signal,
      })

      return data
    },
    staleTime: 60_000,
    gcTime: 300_000,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })

  const revenueCurrency = (revenueSummary as { currency?: string } | null)?.currency ?? 'USD'

  const handlePresetChange = (preset: string) => {
    const [start, end] = preset.split('|')
    setDateRange({ startDate: start, endDate: end })
  }

  const handlePresetSelect = (event: ChangeEvent<HTMLSelectElement>) => {
    handlePresetChange(event.target.value)
  }

  const escapeCsvValue = (value: string | number | null | undefined) => {
    if (value === null || typeof value === 'undefined') return ''
    const stringValue = String(value)
    if (/[",\n]/.test(stringValue)) {
      return `"${stringValue.replace(/"/g, '""')}"`
    }
    return stringValue
  }

  const handleExportCSV = useCallback(() => {
    if (!revenueSummary) {
      setExportError('Nothing to export yet. Try refreshing the dashboard data first.')
      return
    }

    try {
      const csvData = [
        ['Metric', 'Value'],
        ['Total Revenue', formatCurrency(revenueSummary.totalRevenue ?? 0, revenueCurrency)],
        ['Total Impressions', formatNumber(revenueSummary.totalImpressions ?? 0, { maximumFractionDigits: 0 })],
        ['Total Clicks', formatNumber(revenueSummary.totalClicks ?? 0, { maximumFractionDigits: 0 })],
        ['Average eCPM', formatCurrency(revenueSummary.averageEcpm ?? 0, revenueCurrency)],
        ['Average Fill Rate', formatPercentage(revenueSummary?.averageFillRate ?? 0)],
        [
          'Date Range',
          `${dateRange.startDate} to ${dateRange.endDate}`,
        ],
      ]

      const csvContent = csvData.map((row) => row.map(escapeCsvValue).join(',')).join('\r\n')
      const blob = new Blob([csvContent], { type: 'text/csv;charset=utf-8;' })
      const url = URL.createObjectURL(blob)

      const link = document.createElement('a')
      link.href = url
      link.download = `dashboard-export-${dateRange.startDate}-to-${dateRange.endDate}.csv`
      document.body.appendChild(link)
      link.click()
      document.body.removeChild(link)
      URL.revokeObjectURL(url)
      setExportError(null)
    } catch (error) {
      console.error('Failed to export dashboard CSV', error)
      setExportError('Unable to export dashboard data right now. Please try again in a moment.')
    }
  }, [dateRange.endDate, dateRange.startDate, revenueCurrency, revenueSummary])

  const presets = useMemo(() => {
    const now = new Date()
    const format = (date: Date) => date.toISOString().split('T')[0]

    const createPreset = (days: number) => `${format(new Date(now.getTime() - days * 24 * 60 * 60 * 1000))}|${format(now)}`

    return {
      last7: createPreset(7),
      last30: createPreset(30),
      last90: createPreset(90),
    }
  }, [])

  const ecpmChange = revenueSummary?.ecpmChangePercent
  const ecpmTrend: 'up' | 'down' | 'neutral' =
    typeof ecpmChange === 'number'
      ? ecpmChange === 0
        ? 'neutral'
        : ecpmChange > 0
          ? 'up'
          : 'down'
      : 'neutral'

  const revenueSummaryErrorMessage =
    revenueSummaryError instanceof Error
      ? revenueSummaryError.message
      : 'Unable to load revenue summary.'

  const publisherId = session?.user?.publisherId
  const canRenderOperationalWidgets = Boolean(publisherId)

  return (
    <DashboardGate>
    <div className="min-h-screen bg-[#f9fafb]">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="space-y-1">
              <p className="text-xs font-medium text-gray-500">Overview</p>
              <h1 className="text-2xl font-semibold tracking-tight text-gray-900">Dashboard</h1>
              <p className="text-sm text-gray-600">Performance across revenue and delivery.</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <div className="flex items-center gap-2">
                <CalendarRange className="w-5 h-5 text-gray-500" aria-hidden="true" />
                <select
                  aria-label="Select date range"
                  value={`${dateRange.startDate}|${dateRange.endDate}`}
                  onChange={handlePresetSelect}
                  className="h-10 w-48 rounded-lg border border-gray-300 bg-white px-3 text-sm text-gray-900 shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1"
                >
                  <option value={presets.last7}>Last 7 days</option>
                  <option value={presets.last30}>Last 30 days</option>
                  <option value={presets.last90}>Last 90 days</option>
                </select>
              </div>
              <div className="flex flex-col gap-1">
                <button type="button" onClick={handleExportCSV} className="btn btn-outline flex items-center gap-2">
                  <Download className="w-4 h-4" aria-hidden="true" />
                  Export CSV
                </button>
                {exportError && (
                  <p className="text-xs text-red-600" role="alert">
                    {exportError}
                  </p>
                )}
              </div>
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-10">
        <section>
          <h2 className="sr-only">Key performance indicators</h2>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
            {loadingRevenue ? (
              <MetricCardSkeleton count={4} />
            ) : revenueError ? (
              <div className="card border-red-200 bg-red-50 text-red-900 col-span-full">
                <div className="flex flex-col gap-3">
                  <div>
                    <h3 className="text-lg font-semibold">We couldn&apos;t load revenue.</h3>
                    <p className="text-sm">{revenueSummaryErrorMessage}</p>
                  </div>
                  <div>
                    <button
                      type="button"
                      className="btn btn-outline btn-sm"
                      onClick={() => refetchRevenueSummary()}
                    >
                      Retry
                    </button>
                  </div>
                </div>
              </div>
            ) : !revenueSummary ? (
              <div className="card border border-dashed border-gray-200 bg-white text-gray-700 col-span-full">
                <div className="flex flex-col gap-2">
                  <h3 className="text-lg font-semibold">No revenue yet.</h3>
                  <p className="text-sm">
                    We couldn&apos;t find revenue for this window. Adjust the date range or refresh to try again.
                  </p>
                  <div>
                    <button
                      type="button"
                      className="btn btn-outline btn-sm"
                      onClick={() => refetchRevenueSummary()}
                    >
                      Refresh
                    </button>
                  </div>
                </div>
              </div>
            ) : (
              <>
                <MetricCard
                  title="Total Revenue"
                  value={formatCurrency(revenueSummary.totalRevenue ?? 0, revenueCurrency)}
                  change={revenueSummary.revenueChangePercent ?? undefined}
                  icon={<DollarSign className="w-5 h-5" />}
                />
                <MetricCard
                  title="Impressions"
                  value={formatNumber(revenueSummary.totalImpressions ?? 0)}
                  change={revenueSummary.impressionsChangePercent ?? undefined}
                  icon={<Eye className="w-5 h-5" />}
                />
                <MetricCard
                  title="eCPM"
                  value={formatCurrency(revenueSummary.averageEcpm ?? 0, revenueCurrency)}
                  change={ecpmChange}
                  icon={<TrendingUp className="w-5 h-5" />}
                  trend={ecpmTrend}
                />
                <MetricCard
                  title="Fill Rate"
                  value={formatPercentage(revenueSummary.averageFillRate ?? 0)}
                  change={revenueSummary.fillRateChangePercent ?? undefined}
                  icon={<MousePointer className="w-5 h-5" />}
                />
              </>
            )}
          </div>
        </section>

        <section className="space-y-6">
          <h2 className="text-xl font-semibold text-gray-900">Revenue Intelligence</h2>
          <DashboardCharts dateRange={dateRange} currency={revenueCurrency} />
        </section>

        <section>
          <h2 className="sr-only">Operational insights</h2>
          <div className="grid grid-cols-1 xl:grid-cols-3 gap-6">
            {canRenderOperationalWidgets ? (
              <>
                <FraudWidget publisherId={publisherId!} />
                <PayoutWidget />
              </>
            ) : (
              <div className="card col-span-full">
                <h3 className="text-lg font-semibold text-gray-900">Connect a publisher account</h3>
                <p className="text-sm text-gray-600 mt-2">
                  Fraud and payout insights unlock once your session includes an active publisher context. Contact support if
                  you expect to see one here.
                </p>
              </div>
            )}
            <QuickActionsPanel />
          </div>
        </section>
      </main>
    </div>
    </DashboardGate>
  )
}

type QuickAction = {
  label: string
  href: string
  icon: LucideIcon
}

const quickActions: QuickAction[] = [
  {
    label: 'Create Placement',
    href: '/placements/new',
    icon: PlusCircle,
  },
  {
    label: 'Configure Adapters',
    href: '/adapters',
    icon: Settings2,
  },
  {
    label: 'View Analytics',
    href: '/analytics',
    icon: BarChart3,
  },
  {
    label: 'Update Payment Method',
    href: '/settings/payouts',
    icon: CreditCard,
  },
]

function QuickActionsPanel() {
  return (
    <div className="card h-full">
      <div className="flex items-center justify-between mb-4">
        <h3 className="text-lg font-semibold">Quick Actions</h3>
        <span className="text-xs font-medium text-gray-400 uppercase tracking-wide">Shortcuts</span>
      </div>
      <div className="space-y-4">
        {quickActions.map((action) => {
          const Icon = action.icon
          return (
            <Link
              key={action.label}
              href={action.href}
              className="btn btn-outline w-full flex items-start sm:items-center gap-3 py-2.5 text-sm text-left"
            >
              <Icon className="w-4 h-4 text-primary-600 mt-0.5 sm:mt-0" aria-hidden="true" />
              <span className="font-medium text-gray-700 break-words leading-tight">
                {action.label}
              </span>
            </Link>
          )
        })}
      </div>
    </div>
  )
}
