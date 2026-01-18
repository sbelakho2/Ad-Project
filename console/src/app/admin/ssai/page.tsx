'use client'

import { useState, useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
  Monitor,
  Tv,
  Globe,
  Smartphone,
  RefreshCcw,
  CheckCircle2,
  XCircle,
  Clock,
  AlertTriangle,
  Zap,
  ExternalLink,
  TrendingUp,
  TrendingDown,
  Play,
} from 'lucide-react'
import {
  LineChart,
  Line,
  AreaChart,
  Area,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  ComposedChart,
} from 'recharts'
import { apiClient } from '@/lib/api-client'
import { useAdminGate } from '@/lib/useAdminGate'
import { formatNumber, formatPercentage } from '@/lib/utils'

// Types
interface AdapterStatus {
  name: string
  status: 'active' | 'degraded' | 'inactive'
  latency_p95_ms: number
  success_rate: number
  requests_24h: number
  revenue_24h: number
  timeout_rate: number
}

interface SSAIMetrics {
  total_sessions_24h: number
  active_sessions: number
  avg_session_duration_sec: number
  ad_fill_rate: number
  beacons_fired_24h: number
  beacon_success_rate: number
  midroll_completion_rate: number
  preroll_completion_rate: number
}

interface PlatformMetrics {
  platform: string
  sessions_24h: number
  fill_rate: number
  completion_rate: number
  avg_latency_ms: number
  revenue: number
}

type TimeSeriesPoint = {
  time: string
  sessions: number
  fillRate: number
  revenue: number
}

type AdminDashboardResponse = {
  adapters: AdapterStatus[]
  platformMetrics: PlatformMetrics[]
  timeSeries: TimeSeriesPoint[]
  ssai?: {
    safeMode?: { active: boolean; reason: string }
    counters?: Record<string, number>
    metrics?: SSAIMetrics
    killSwitches?: Array<{ key: string; active: boolean; reason: string; activatedAt: number; activatedBy: string }>
    rollouts?: Array<{ key: string; enabled: boolean; percentage: number; tenants: string[] }>
  }
}

async function fetchAdminDashboard(): Promise<AdminDashboardResponse> {
  const res = await apiClient.get('/ssai/admin/dashboard')
  return res.data as AdminDashboardResponse
}

// Colors
const COLORS = {
  primary: '#3b82f6',
  success: '#22c55e',
  warning: '#f59e0b',
  danger: '#ef4444',
  purple: '#8b5cf6',
  cyan: '#06b6d4',
}

export default function AdminSSAIPage() {
  const { isOperator, isLoading: gateLoading } = useAdminGate()
  const [refreshKey, setRefreshKey] = useState(0)

  const {
    data: dashboard,
    isLoading: dashboardLoading,
    refetch: refetchDashboard,
  } = useQuery({
    queryKey: ['admin-ssai-dashboard', refreshKey],
    queryFn: fetchAdminDashboard,
    refetchInterval: 30000,
    enabled: isOperator,
  })

  const adapters = dashboard?.adapters ?? []
  const platformMetrics = dashboard?.platformMetrics ?? []
  const timeSeriesData = useMemo(() => dashboard?.timeSeries ?? [], [dashboard])
  const ssaiMetrics = useMemo<SSAIMetrics | null>(() => dashboard?.ssai?.metrics ?? null, [dashboard])

  const handleRefresh = () => {
    setRefreshKey((k) => k + 1)
    refetchDashboard()
  }

  if (gateLoading) {
    return (
      <div className="min-h-[50vh] flex items-center justify-center">
        <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-primary-500" />
      </div>
    )
  }

  if (!isOperator) {
    return (
      <div className="text-center py-12">
        <p className="text-red-600">Access denied. Operator authentication required.</p>
      </div>
    )
  }

  const isLoading = dashboardLoading

  // Aggregate metrics
  const totalRequests = adapters.reduce((sum, a) => sum + a.requests_24h, 0)
  const totalRevenue = adapters.reduce((sum, a) => sum + a.revenue_24h, 0)
  const avgSuccessRate = adapters.length
    ? adapters.reduce((sum, a) => sum + a.success_rate, 0) / adapters.length
    : 0

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-semibold text-gray-900">SSAI & Adapters</h2>
          <p className="text-sm text-gray-500 mt-1">
            Server-side ad insertion monitoring and adapter performance
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-gray-500">
            Last updated: {new Date().toLocaleTimeString()}
          </span>
          <button
            onClick={handleRefresh}
            disabled={isLoading}
            className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-lg hover:bg-gray-50 disabled:opacity-50"
          >
            <RefreshCcw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />
            Refresh
          </button>
        </div>
      </div>

      {/* SSAI Key Metrics */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <MetricCard
          title="Active Sessions"
          value={ssaiMetrics?.active_sessions?.toLocaleString() ?? '--'}
          subtitle="Currently streaming"
          icon={Play}
          color="primary"
          loading={dashboardLoading}
        />
        <MetricCard
          title="Ad Fill Rate"
          value={ssaiMetrics?.ad_fill_rate ? `${(ssaiMetrics.ad_fill_rate * 100).toFixed(1)}%` : '--'}
          subtitle="24h average"
          icon={Zap}
          color="success"
          loading={dashboardLoading}
        />
        <MetricCard
          title="Beacons Success"
          value={ssaiMetrics?.beacon_success_rate ? `${(ssaiMetrics.beacon_success_rate * 100).toFixed(2)}%` : '--'}
          subtitle="Tracking accuracy"
          icon={CheckCircle2}
          color="purple"
          loading={dashboardLoading}
        />
        <MetricCard
          title="Avg Session Length"
          value={ssaiMetrics?.avg_session_duration_sec ? `${Math.floor(ssaiMetrics.avg_session_duration_sec / 60)}m ${ssaiMetrics.avg_session_duration_sec % 60}s` : '--'}
          subtitle="Engagement time"
          icon={Clock}
          color="cyan"
          loading={dashboardLoading}
        />
      </div>

      {/* Session & Revenue Charts */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Session Volume (24h)</h3>
          <div className="h-64">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={timeSeriesData}>
                <defs>
                  <linearGradient id="sessionsGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={COLORS.primary} stopOpacity={0.2} />
                    <stop offset="95%" stopColor={COLORS.primary} stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                <XAxis dataKey="time" tick={{ fontSize: 11 }} stroke="#9ca3af" />
                <YAxis tick={{ fontSize: 11 }} stroke="#9ca3af" />
                <Tooltip
                  formatter={(value: number) => [value.toLocaleString(), 'Sessions']}
                  contentStyle={{ borderRadius: '8px', border: '1px solid #e5e7eb' }}
                />
                <Area
                  type="monotone"
                  dataKey="sessions"
                  stroke={COLORS.primary}
                  fill="url(#sessionsGradient)"
                  strokeWidth={2}
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </div>

        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Fill Rate & Revenue (24h)</h3>
          <div className="h-64">
            <ResponsiveContainer width="100%" height="100%">
              <ComposedChart data={timeSeriesData}>
                <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                <XAxis dataKey="time" tick={{ fontSize: 11 }} stroke="#9ca3af" />
                <YAxis
                  yAxisId="left"
                  tick={{ fontSize: 11 }}
                  stroke="#9ca3af"
                  tickFormatter={(v) => `${(v * 100).toFixed(0)}%`}
                  domain={[0.7, 1]}
                />
                <YAxis
                  yAxisId="right"
                  orientation="right"
                  tick={{ fontSize: 11 }}
                  stroke="#9ca3af"
                  tickFormatter={(v) => `$${v}`}
                />
                <Tooltip
                  formatter={(value: number, name: string) => [
                    name === 'fillRate' ? `${(value * 100).toFixed(1)}%` : `$${value}`,
                    name === 'fillRate' ? 'Fill Rate' : 'Revenue',
                  ]}
                  contentStyle={{ borderRadius: '8px', border: '1px solid #e5e7eb' }}
                />
                <Legend />
                <Line
                  yAxisId="left"
                  type="monotone"
                  dataKey="fillRate"
                  stroke={COLORS.success}
                  strokeWidth={2}
                  dot={false}
                  name="Fill Rate"
                />
                <Bar
                  yAxisId="right"
                  dataKey="revenue"
                  fill={COLORS.purple}
                  radius={[2, 2, 0, 0]}
                  name="Revenue"
                />
              </ComposedChart>
            </ResponsiveContainer>
          </div>
        </div>
      </div>

      {/* Platform Performance */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">Platform Performance</h3>
        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b">
                <th className="text-left py-3 px-4 text-sm font-medium text-gray-500">Platform</th>
                <th className="text-right py-3 px-4 text-sm font-medium text-gray-500">Sessions (24h)</th>
                <th className="text-right py-3 px-4 text-sm font-medium text-gray-500">Fill Rate</th>
                <th className="text-right py-3 px-4 text-sm font-medium text-gray-500">Completion</th>
                <th className="text-right py-3 px-4 text-sm font-medium text-gray-500">Avg Latency</th>
                <th className="text-right py-3 px-4 text-sm font-medium text-gray-500">Revenue</th>
              </tr>
            </thead>
            <tbody>
              {isLoading ? (
                <tr>
                  <td colSpan={6} className="py-8 text-center text-gray-500">Loading...</td>
                </tr>
              ) : (
                platformMetrics?.map((p) => (
                  <tr key={p.platform} className="border-b last:border-0 hover:bg-gray-50">
                    <td className="py-3 px-4">
                      <div className="flex items-center gap-2">
                        <PlatformIcon platform={p.platform} />
                        <span className="font-medium text-gray-900">{p.platform}</span>
                      </div>
                    </td>
                    <td className="text-right py-3 px-4 text-gray-700">
                      {p.sessions_24h.toLocaleString()}
                    </td>
                    <td className="text-right py-3 px-4">
                      <span className={p.fill_rate >= 0.9 ? 'text-green-600' : 'text-yellow-600'}>
                        {(p.fill_rate * 100).toFixed(1)}%
                      </span>
                    </td>
                    <td className="text-right py-3 px-4">
                      <span className={p.completion_rate >= 0.85 ? 'text-green-600' : 'text-yellow-600'}>
                        {(p.completion_rate * 100).toFixed(1)}%
                      </span>
                    </td>
                    <td className="text-right py-3 px-4">
                      <span className={p.avg_latency_ms <= 50 ? 'text-green-600' : 'text-yellow-600'}>
                        {p.avg_latency_ms}ms
                      </span>
                    </td>
                    <td className="text-right py-3 px-4 font-medium text-gray-900">
                      ${p.revenue.toLocaleString()}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* Adapter Status */}
      <div className="bg-white rounded-xl border p-6">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-semibold text-gray-900">Ad Server Adapters</h3>
          <div className="flex items-center gap-4 text-sm">
            <div className="flex items-center gap-2">
              <div className="w-3 h-3 rounded-full bg-green-500" />
              <span className="text-gray-600">Active</span>
            </div>
            <div className="flex items-center gap-2">
              <div className="w-3 h-3 rounded-full bg-yellow-500" />
              <span className="text-gray-600">Degraded</span>
            </div>
            <div className="flex items-center gap-2">
              <div className="w-3 h-3 rounded-full bg-red-500" />
              <span className="text-gray-600">Inactive</span>
            </div>
          </div>
        </div>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {isLoading ? (
            <div className="col-span-full py-8 text-center text-gray-500">Loading adapters...</div>
          ) : (
            adapters?.map((adapter) => (
              <AdapterCard key={adapter.name} adapter={adapter} />
            ))
          )}
        </div>
      </div>

      {/* Completion Rate Summary */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Ad Completion Rates</h3>
          <div className="space-y-4">
            <CompletionBar
              label="Preroll Completion"
              value={ssaiMetrics?.preroll_completion_rate ?? 0}
              color={COLORS.success}
            />
            <CompletionBar
              label="Midroll Completion"
              value={ssaiMetrics?.midroll_completion_rate ?? 0}
              color={COLORS.primary}
            />
          </div>
        </div>

        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Aggregate Stats (24h)</h3>
          <div className="grid grid-cols-2 gap-4">
            <div className="p-4 bg-gray-50 rounded-lg">
              <p className="text-sm text-gray-500">Total Sessions</p>
              <p className="text-2xl font-semibold text-gray-900">
                {ssaiMetrics?.total_sessions_24h?.toLocaleString() ?? '--'}
              </p>
            </div>
            <div className="p-4 bg-gray-50 rounded-lg">
              <p className="text-sm text-gray-500">Total Requests</p>
              <p className="text-2xl font-semibold text-gray-900">
                {totalRequests.toLocaleString()}
              </p>
            </div>
            <div className="p-4 bg-gray-50 rounded-lg">
              <p className="text-sm text-gray-500">Beacons Fired</p>
              <p className="text-2xl font-semibold text-gray-900">
                {ssaiMetrics?.beacons_fired_24h?.toLocaleString() ?? '--'}
              </p>
            </div>
            <div className="p-4 bg-green-50 rounded-lg">
              <p className="text-sm text-gray-500">Adapter Revenue</p>
              <p className="text-2xl font-semibold text-green-600">
                ${totalRevenue.toLocaleString()}
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="text-center text-xs text-gray-400 py-4">
        SSAI metrics refresh every 30 seconds • Adapter data refreshes every 60 seconds
      </div>
    </div>
  )
}

// Components
function MetricCard({
  title,
  value,
  subtitle,
  icon: Icon,
  color,
  loading,
}: {
  title: string
  value: string
  subtitle: string
  icon: React.ComponentType<{ className?: string }>
  color: 'primary' | 'success' | 'warning' | 'danger' | 'purple' | 'cyan'
  loading?: boolean
}) {
  const colorClasses = {
    primary: 'text-blue-600 bg-blue-50',
    success: 'text-green-600 bg-green-50',
    warning: 'text-yellow-600 bg-yellow-50',
    danger: 'text-red-600 bg-red-50',
    purple: 'text-purple-600 bg-purple-50',
    cyan: 'text-cyan-600 bg-cyan-50',
  }

  return (
    <div className="bg-white rounded-xl border p-5">
      <div className="flex items-start justify-between">
        <div>
          <p className="text-sm text-gray-500">{title}</p>
          {loading ? (
            <div className="h-8 w-24 bg-gray-100 animate-pulse rounded mt-1" />
          ) : (
            <p className="text-2xl font-semibold text-gray-900 mt-1">{value}</p>
          )}
          <p className="text-xs text-gray-400 mt-1">{subtitle}</p>
        </div>
        <div className={`p-2 rounded-lg ${colorClasses[color]}`}>
          <Icon className="h-5 w-5" />
        </div>
      </div>
    </div>
  )
}

function PlatformIcon({ platform }: { platform: string }) {
  const iconMap: Record<string, React.ReactNode> = {
    Roku: <Tv className="h-4 w-4 text-purple-500" />,
    'Fire TV': <Tv className="h-4 w-4 text-orange-500" />,
    'Apple TV': <Tv className="h-4 w-4 text-gray-700" />,
    'Android TV': <Tv className="h-4 w-4 text-green-500" />,
    'Samsung TV': <Tv className="h-4 w-4 text-blue-500" />,
    'Web Video': <Globe className="h-4 w-4 text-cyan-500" />,
  }
  return iconMap[platform] || <Monitor className="h-4 w-4 text-gray-500" />
}

function AdapterCard({ adapter }: { adapter: AdapterStatus }) {
  const statusColor =
    adapter.status === 'active'
      ? 'bg-green-500'
      : adapter.status === 'degraded'
      ? 'bg-yellow-500'
      : 'bg-red-500'

  return (
    <div className="border rounded-lg p-4">
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <div className={`w-3 h-3 rounded-full ${statusColor}`} />
          <span className="font-medium text-gray-900">{adapter.name}</span>
        </div>
        <span
          className={`text-xs font-medium ${
            adapter.status === 'active'
              ? 'text-green-600'
              : adapter.status === 'degraded'
              ? 'text-yellow-600'
              : 'text-red-600'
          }`}
        >
          {adapter.status}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 text-sm">
        <div>
          <p className="text-gray-500">Success Rate</p>
          <p className={adapter.success_rate >= 0.98 ? 'text-green-600 font-medium' : 'text-yellow-600 font-medium'}>
            {(adapter.success_rate * 100).toFixed(2)}%
          </p>
        </div>
        <div>
          <p className="text-gray-500">P95 Latency</p>
          <p className={adapter.latency_p95_ms <= 60 ? 'text-green-600 font-medium' : 'text-yellow-600 font-medium'}>
            {adapter.latency_p95_ms}ms
          </p>
        </div>
        <div>
          <p className="text-gray-500">Requests (24h)</p>
          <p className="text-gray-700">{(adapter.requests_24h / 1000).toFixed(1)}K</p>
        </div>
        <div>
          <p className="text-gray-500">Timeout Rate</p>
          <p className={adapter.timeout_rate < 0.01 ? 'text-green-600 font-medium' : 'text-yellow-600 font-medium'}>
            {(adapter.timeout_rate * 100).toFixed(2)}%
          </p>
        </div>
      </div>
    </div>
  )
}

function CompletionBar({
  label,
  value,
  color,
}: {
  label: string
  value: number
  color: string
}) {
  return (
    <div>
      <div className="flex justify-between mb-1">
        <span className="text-sm text-gray-700">{label}</span>
        <span className="text-sm font-medium text-gray-900">{(value * 100).toFixed(1)}%</span>
      </div>
      <div className="h-3 bg-gray-200 rounded-full overflow-hidden">
        <div
          className="h-full rounded-full transition-all duration-500"
          style={{ width: `${value * 100}%`, backgroundColor: color }}
        />
      </div>
    </div>
  )
}
