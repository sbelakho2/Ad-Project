'use client'

import { useState, useMemo, useId } from 'react'
import { useQuery } from '@tanstack/react-query'
import { revenueApi } from '@/lib/api'
import { BarChart3, TrendingUp, Download, DollarSign, Eye, MousePointer, Zap } from 'lucide-react'
import { formatCurrency, formatNumber, formatPercentage, getLocale } from '@/lib/utils'
import {
  LineChart,
  Line,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
} from 'recharts'

export default function AnalyticsPage() {
  const defaultStart = useMemo(
    () => new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString().split('T')[0],
    []
  )
  const today = useMemo(() => new Date().toISOString().split('T')[0], [])

  const [dateRange, setDateRange] = useState({
    startDate: defaultStart,
    endDate: today,
  })
  const [granularity, setGranularity] = useState<'hour' | 'day'>('day')
  const locale = useMemo(() => getLocale(), [])
  const chartDateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(locale, {
        month: 'short',
        day: 'numeric',
      }),
    [locale]
  )
  const csvDateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(locale, {
        dateStyle: 'medium',
      }),
    [locale]
  )
  const csvNumberFormatter = useMemo(() => new Intl.NumberFormat(locale), [locale])
  const csvPercentFormatter = useMemo(
    () =>
      new Intl.NumberFormat(locale, {
        style: 'percent',
        maximumFractionDigits: 2,
      }),
    [locale]
  )
  const revenueTrendDescriptionId = useId()
  const efficiencyDescriptionId = useId()
  const volumeDescriptionId = useId()

  const {
    data: timeSeries,
    isLoading: loadingTimeSeries,
    isError: timeSeriesError,
    error: timeSeriesErrorObject,
    refetch: refetchTimeSeries,
  } = useQuery({
    queryKey: ['revenue-timeseries', dateRange.startDate, dateRange.endDate, granularity],
    queryFn: async ({ signal }) => {
      const { data } = await revenueApi.getTimeSeries({
        startDate: dateRange.startDate,
        endDate: dateRange.endDate,
        granularity,
        signal,
      })
      return data
    },
    staleTime: 60_000,
    gcTime: 300_000,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })

  const {
    data: summary,
    isLoading: loadingSummary,
    isError: summaryError,
    error: summaryErrorObject,
    refetch: refetchSummary,
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

  const handlePresetChange = (preset: string) => {
    const [start, end] = preset.split('|')
    setDateRange({ startDate: start, endDate: end })
  }

  const chartData = useMemo(() => {
    if (!Array.isArray(timeSeries) || timeSeries.length === 0) return []
    return timeSeries.map((item) => {
      const parsedDate = new Date(item.date)
      return {
        date: chartDateFormatter.format(parsedDate),
        revenue: item.revenue,
        impressions: item.impressions / 1000, // Scale down for chart display
        clicks: item.clicks,
        ecpm: item.ecpm,
        fillRate: item.fillRate * 100,
      }
    })
  }, [chartDateFormatter, timeSeries])

  const escapeCsvValue = (value: string | number) => {
    const stringValue = String(value ?? '')
    return /[",\n]/.test(stringValue) ? `"${stringValue.replace(/"/g, '""')}"` : stringValue
  }

  const handleExportReport = () => {
    if (!Array.isArray(timeSeries) || timeSeries.length === 0 || !summary) return

    const rows = [
      ['Date', 'Revenue', 'Impressions', 'Clicks', 'eCPM', 'Fill Rate'],
      ...timeSeries.map((item) => [
        csvDateFormatter.format(new Date(item.date)),
        formatCurrency(item.revenue),
        csvNumberFormatter.format(item.impressions),
        csvNumberFormatter.format(item.clicks),
        formatCurrency(item.ecpm),
        csvPercentFormatter.format(item.fillRate),
      ]),
      [],
      ['Summary'],
      ['Total Revenue', formatCurrency(summary.totalRevenue)],
      ['Total Impressions', csvNumberFormatter.format(summary.totalImpressions)],
      ['Total Clicks', csvNumberFormatter.format(summary.totalClicks)],
      ['Average eCPM', formatCurrency(summary.averageEcpm)],
      ['Average Fill Rate', csvPercentFormatter.format(summary.averageFillRate)],
    ]

    const csvContent = rows.map((row) => row.map(escapeCsvValue).join(',')).join('\n')
    const blob = new Blob([csvContent], { type: 'text/csv;charset=utf-8;' })
    const link = document.createElement('a')
    const url = URL.createObjectURL(blob)
    
    link.setAttribute('href', url)
    link.setAttribute('download', `analytics-report-${dateRange.startDate}-to-${dateRange.endDate}.csv`)
    link.style.visibility = 'hidden'
    document.body.appendChild(link)
    link.click()
    document.body.removeChild(link)
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="h-12 w-12 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center">
                <BarChart3 className="h-6 w-6" aria-hidden={true} />
              </div>
              <div>
                <p className="text-sm font-medium text-primary-600">Insights</p>
                <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Analytics</h1>
                <p className="text-sm text-gray-600 mt-1">
                  Deep dive into revenue trends, performance metrics, and optimization opportunities.
                </p>
              </div>
            </div>
            <button onClick={handleExportReport} className="btn btn-outline flex items-center gap-2">
              <Download className="h-4 w-4" aria-hidden={true} />
              Export Report
            </button>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-6">
        {/* Date range controls */}
        <div className="card">
          <div className="flex flex-col md:flex-row md:items-center gap-4">
            <div className="flex-1 grid grid-cols-2 gap-3">
              <div>
                <label htmlFor="startDate" className="label">
                  Start Date
                </label>
                <input
                  id="startDate"
                  type="date"
                  className="input"
                  value={dateRange.startDate}
                  onChange={(e) => setDateRange({ ...dateRange, startDate: e.target.value })}
                />
              </div>
              <div>
                <label htmlFor="endDate" className="label">
                  End Date
                </label>
                <input
                  id="endDate"
                  type="date"
                  className="input"
                  value={dateRange.endDate}
                  onChange={(e) => setDateRange({ ...dateRange, endDate: e.target.value })}
                />
              </div>
            </div>
            <div className="flex gap-2">
              <button
                onClick={() =>
                  handlePresetChange(
                    `${new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString().split('T')[0]}|${today}`
                  )
                }
                className="btn btn-outline text-sm"
              >
                Last 7 Days
              </button>
              <button
                onClick={() =>
                  handlePresetChange(
                    `${new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString().split('T')[0]}|${today}`
                  )
                }
                className="btn btn-outline text-sm"
              >
                Last 30 Days
              </button>
            </div>
          </div>
        </div>

        {/* Summary metrics */}
        {loadingSummary ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
            {[1, 2, 3, 4].map((i) => (
              <div key={i} className="card animate-pulse">
                <div className="h-4 bg-gray-200 rounded w-1/2 mb-3" />
                <div className="h-8 bg-gray-200 rounded w-3/4" />
              </div>
            ))}
          </div>
        ) : summaryError ? (
          <div className="card border-danger-200 bg-danger-50 text-danger-700" role="alert">
            <p className="font-semibold">We couldn’t load the analytics summary.</p>
            <p className="text-sm mt-1">
              {summaryErrorObject instanceof Error ? summaryErrorObject.message : 'Please try again later.'}
            </p>
            <button
              type="button"
              onClick={() => refetchSummary()}
              className="mt-3 inline-flex items-center text-sm font-semibold text-danger-700 underline"
            >
              Retry summary fetch
            </button>
          </div>
        ) : summary ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
            <div className="card">
              <div className="flex items-center justify-between mb-2">
                <p className="text-sm font-medium text-gray-600">Total Revenue</p>
                <DollarSign className="h-5 w-5 text-gray-400" aria-hidden={true} />
              </div>
              <p className="text-2xl font-bold text-gray-900">{formatCurrency(summary.totalRevenue)}</p>
              {summary.revenueChangePercent !== undefined && (
                <p
                  className={`text-xs mt-1 flex items-center gap-1 ${
                    summary.revenueChangePercent >= 0 ? 'text-success-600' : 'text-danger-600'
                  }`}
                >
                  <TrendingUp
                    className={`h-3 w-3 ${summary.revenueChangePercent < 0 ? 'rotate-180' : ''}`}
                    aria-hidden={true}
                  />
                  {formatPercentage(Math.abs(summary.revenueChangePercent))} vs prev period
                </p>
              )}
            </div>

            <div className="card">
              <div className="flex items-center justify-between mb-2">
                <p className="text-sm font-medium text-gray-600">Impressions</p>
                <Eye className="h-5 w-5 text-gray-400" aria-hidden={true} />
              </div>
              <p className="text-2xl font-bold text-gray-900">{formatNumber(summary.totalImpressions)}</p>
              {summary.impressionsChangePercent !== undefined && (
                <p
                  className={`text-xs mt-1 flex items-center gap-1 ${
                    summary.impressionsChangePercent >= 0 ? 'text-success-600' : 'text-danger-600'
                  }`}
                >
                  <TrendingUp
                    className={`h-3 w-3 ${summary.impressionsChangePercent < 0 ? 'rotate-180' : ''}`}
                    aria-hidden={true}
                  />
                  {formatPercentage(Math.abs(summary.impressionsChangePercent))} vs prev period
                </p>
              )}
            </div>

            <div className="card">
              <div className="flex items-center justify-between mb-2">
                <p className="text-sm font-medium text-gray-600">Avg eCPM</p>
                <Zap className="h-5 w-5 text-gray-400" aria-hidden={true} />
              </div>
              <p className="text-2xl font-bold text-gray-900">{formatCurrency(summary.averageEcpm)}</p>
              {summary.ecpmChangePercent !== undefined && (
                <p
                  className={`text-xs mt-1 flex items-center gap-1 ${
                    summary.ecpmChangePercent >= 0 ? 'text-success-600' : 'text-danger-600'
                  }`}
                >
                  <TrendingUp
                    className={`h-3 w-3 ${summary.ecpmChangePercent < 0 ? 'rotate-180' : ''}`}
                    aria-hidden={true}
                  />
                  {formatPercentage(Math.abs(summary.ecpmChangePercent))} vs prev period
                </p>
              )}
            </div>

            <div className="card">
              <div className="flex items-center justify-between mb-2">
                <p className="text-sm font-medium text-gray-600">Fill Rate</p>
                <MousePointer className="h-5 w-5 text-gray-400" aria-hidden={true} />
              </div>
              <p className="text-2xl font-bold text-gray-900">{formatPercentage(summary.averageFillRate)}</p>
              {summary.fillRateChangePercent !== undefined && (
                <p
                  className={`text-xs mt-1 flex items-center gap-1 ${
                    summary.fillRateChangePercent >= 0 ? 'text-success-600' : 'text-danger-600'
                  }`}
                >
                  <TrendingUp
                    className={`h-3 w-3 ${summary.fillRateChangePercent < 0 ? 'rotate-180' : ''}`}
                    aria-hidden={true}
                  />
                  {formatPercentage(Math.abs(summary.fillRateChangePercent))} vs prev period
                </p>
              )}
            </div>
          </div>
        ) : (
          <div className="card text-sm text-gray-600">No summary data available for the selected period.</div>
        )}

        {/* Revenue over time */}
        <div className="card">
          <div className="flex items-center justify-between mb-6">
            <h2 className="text-lg font-semibold text-gray-900" id="revenue-trend-heading">
              Revenue Trend
            </h2>
            <div className="flex gap-2">
              {(['hour', 'day'] as const).map((g) => (
                <button
                  key={g}
                  onClick={() => setGranularity(g)}
                  className={`px-3 py-1 rounded text-sm font-medium ${
                    granularity === g ? 'bg-primary-600 text-white' : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
                  }`}
                >
                  {g.charAt(0).toUpperCase() + g.slice(1)}
                </button>
              ))}
            </div>
          </div>
          {timeSeriesError ? (
            <div className="h-80 flex flex-col items-center justify-center text-center text-danger-700">
              <p className="font-semibold">We couldn’t load the revenue trend.</p>
              <p className="text-sm mt-1">
                {timeSeriesErrorObject instanceof Error ? timeSeriesErrorObject.message : 'Please try again later.'}
              </p>
              <button
                type="button"
                onClick={() => refetchTimeSeries()}
                className="mt-3 inline-flex items-center text-sm font-semibold text-danger-700 underline"
              >
                Retry trend fetch
              </button>
            </div>
          ) : loadingTimeSeries ? (
            <div className="h-80 flex items-center justify-center">
              <div className="animate-pulse text-gray-400">Loading chart...</div>
            </div>
          ) : chartData.length > 0 ? (
            <div role="figure" aria-labelledby="revenue-trend-heading" aria-describedby={revenueTrendDescriptionId}>
              <p id={revenueTrendDescriptionId} className="sr-only">
                {`Line chart showing revenue from ${chartData[0].date} to ${chartData[chartData.length - 1].date}.`}
              </p>
              <ResponsiveContainer width="100%" height={320}>
                <LineChart data={chartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                  <XAxis dataKey="date" stroke="#6b7280" style={{ fontSize: '12px' }} />
                  <YAxis stroke="#6b7280" style={{ fontSize: '12px' }} />
                  <Tooltip
                    contentStyle={{
                      backgroundColor: 'white',
                      border: '1px solid #e5e7eb',
                      borderRadius: '8px',
                      padding: '12px',
                    }}
                  />
                  <Legend wrapperStyle={{ fontSize: '14px' }} />
                  <Line
                    type="monotone"
                    dataKey="revenue"
                    stroke="#2563eb"
                    strokeWidth={2}
                    name="Revenue"
                    dot={{ fill: '#2563eb', r: 3 }}
                  />
                </LineChart>
              </ResponsiveContainer>
            </div>
          ) : (
            <div className="h-80 flex items-center justify-center text-gray-500">
              No data available for selected period
            </div>
          )}
        </div>

        {/* Performance metrics */}
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <div className="card">
            <h2 className="text-lg font-semibold text-gray-900 mb-6" id="efficiency-heading">
              eCPM & Fill Rate
            </h2>
            {timeSeriesError ? (
              <div className="h-64 flex flex-col items-center justify-center text-center text-danger-700">
                <p className="font-semibold">Unable to load efficiency metrics.</p>
                <p className="text-sm mt-1">
                  {timeSeriesErrorObject instanceof Error ? timeSeriesErrorObject.message : 'Please try again.'}
                </p>
                <button
                  type="button"
                  onClick={() => refetchTimeSeries()}
                  className="mt-3 inline-flex items-center text-sm font-semibold text-danger-700 underline"
                >
                  Retry
                </button>
              </div>
            ) : loadingTimeSeries ? (
              <div className="h-64 flex items-center justify-center">
                <div className="animate-pulse text-gray-400">Loading chart...</div>
              </div>
            ) : chartData.length > 0 ? (
              <div role="figure" aria-labelledby="efficiency-heading" aria-describedby={efficiencyDescriptionId}>
                <p id={efficiencyDescriptionId} className="sr-only">
                  {`Line chart comparing eCPM and fill rate for ${chartData.length} data points.`}
                </p>
                <ResponsiveContainer width="100%" height={256}>
                  <LineChart data={chartData}>
                    <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                    <XAxis dataKey="date" stroke="#6b7280" style={{ fontSize: '12px' }} />
                    <YAxis stroke="#6b7280" style={{ fontSize: '12px' }} />
                    <Tooltip
                      contentStyle={{
                        backgroundColor: 'white',
                        border: '1px solid #e5e7eb',
                        borderRadius: '8px',
                        padding: '12px',
                      }}
                    />
                    <Legend wrapperStyle={{ fontSize: '14px' }} />
                    <Line
                      type="monotone"
                      dataKey="ecpm"
                      stroke="#10b981"
                      strokeWidth={2}
                      name="eCPM"
                      dot={{ fill: '#10b981', r: 3 }}
                    />
                    <Line
                      type="monotone"
                      dataKey="fillRate"
                      stroke="#f59e0b"
                      strokeWidth={2}
                      name="Fill Rate (%)"
                      dot={{ fill: '#f59e0b', r: 3 }}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>
            ) : (
              <div className="h-64 flex items-center justify-center text-gray-500">
                No data available
              </div>
            )}
          </div>

          <div className="card">
            <h2 className="text-lg font-semibold text-gray-900 mb-6" id="volume-heading">
              Impressions & Clicks
            </h2>
            {timeSeriesError ? (
              <div className="h-64 flex flex-col items-center justify-center text-center text-danger-700">
                <p className="font-semibold">Unable to load volume metrics.</p>
                <p className="text-sm mt-1">
                  {timeSeriesErrorObject instanceof Error ? timeSeriesErrorObject.message : 'Please try again.'}
                </p>
                <button
                  type="button"
                  onClick={() => refetchTimeSeries()}
                  className="mt-3 inline-flex items-center text-sm font-semibold text-danger-700 underline"
                >
                  Retry
                </button>
              </div>
            ) : loadingTimeSeries ? (
              <div className="h-64 flex items-center justify-center">
                <div className="animate-pulse text-gray-400">Loading chart...</div>
              </div>
            ) : chartData.length > 0 ? (
              <div role="figure" aria-labelledby="volume-heading" aria-describedby={volumeDescriptionId}>
                <p id={volumeDescriptionId} className="sr-only">
                  {`Bar chart comparing impressions (thousands) and clicks for ${chartData.length} dates.`}
                </p>
                <ResponsiveContainer width="100%" height={256}>
                  <BarChart data={chartData}>
                    <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                    <XAxis dataKey="date" stroke="#6b7280" style={{ fontSize: '12px' }} />
                    <YAxis stroke="#6b7280" style={{ fontSize: '12px' }} />
                    <Tooltip
                      contentStyle={{
                        backgroundColor: 'white',
                        border: '1px solid #e5e7eb',
                        borderRadius: '8px',
                        padding: '12px',
                      }}
                    />
                    <Legend wrapperStyle={{ fontSize: '14px' }} />
                    <Bar dataKey="impressions" fill="#3b82f6" name="Impressions (K)" />
                    <Bar dataKey="clicks" fill="#8b5cf6" name="Clicks" />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            ) : (
              <div className="h-64 flex items-center justify-center text-gray-500">
                No data available
              </div>
            )}
          </div>
        </div>
      </main>
    </div>
  )
}
