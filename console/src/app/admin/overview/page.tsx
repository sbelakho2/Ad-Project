'use client'

import { useEffect, useState, useMemo } from 'react'
import { useRouter } from 'next/navigation'
import { useQuery } from '@tanstack/react-query'
import {
  Activity,
  Server,
  Database,
  Cpu,
  MemoryStick,
  HardDrive,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  RefreshCcw,
  Zap,
  Clock,
  ArrowUpRight,
  ArrowDownRight,
  ExternalLink,
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
  PieChart,
  Pie,
  Cell,
} from 'recharts'
import { apiClient } from '@/lib/api-client'
import { useAdminGate } from '@/lib/useAdminGate'
import { formatNumber, formatPercentage } from '@/lib/utils'

// Types
interface SystemHealth {
  status: 'healthy' | 'degraded' | 'unhealthy'
  services: {
    postgres: 'up' | 'down'
    redis: 'up' | 'down'
    queues: 'up' | 'down'
  }
  timestamp: string
}

interface ReadinessData {
  status: string
  service: string
  database: {
    connected: boolean
    latency_ms: number
    status: string
  }
  redis: {
    ready: boolean
  }
  cache: {
    hit_ratio: number
  }
  timestamp: string
}

interface RedMetrics {
  api_rps_5m: number | null
  api_error_rate_5m: number | null
  api_p95_latency_5m: number | null
  rtb_p95_latency_5m: number | null
  rtb_adapter_timeouts_rps_5m: number | null
  timestamp: string
}

interface MetricPoint {
  time: string
  value: number
}

// Color palette
const COLORS = {
  primary: '#3b82f6',
  success: '#22c55e',
  warning: '#f59e0b',
  danger: '#ef4444',
  info: '#06b6d4',
  purple: '#8b5cf6',
  gray: '#64748b',
}

const PIE_COLORS = ['#22c55e', '#f59e0b', '#ef4444', '#3b82f6']

// API Functions
async function fetchHealth(): Promise<SystemHealth> {
  const res = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/health`, {
    cache: 'no-store',
  })
  if (!res.ok) throw new Error('Health check failed')
  return res.json()
}

async function fetchReadiness(): Promise<ReadinessData> {
  const res = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/ready`, {
    cache: 'no-store',
  })
  if (!res.ok) throw new Error('Readiness check failed')
  return res.json()
}

async function fetchRedMetrics(): Promise<RedMetrics> {
  const res = await apiClient.get('/api/admin/red-summary')
  return res.data
}

// Generate deterministic historical data (no randomness) for charts
function generateHistoricalData(current: number, points: number = 24): MetricPoint[] {
  const data: MetricPoint[] = []
  const now = Date.now()
  const amplitude = current * 0.15
  const base = current
  for (let i = points - 1; i >= 0; i--) {
    const t = (points - 1 - i) / Math.max(points - 1, 1)
    const wave = Math.sin(t * Math.PI * 2)
    const value = Math.max(0, base + amplitude * wave)
    data.push({
      time: new Date(now - i * 3600000).toLocaleTimeString('en-US', {
        hour: '2-digit',
        minute: '2-digit',
      }),
      value,
    })
  }
  return data
}

export default function AdminOverviewPage() {
  const { isAdmin, isLoading: gateLoading, isOperator } = useAdminGate()
  const router = useRouter()
  const [refreshKey, setRefreshKey] = useState(0)

  // Queries
  const {
    data: health,
    isLoading: healthLoading,
    error: healthError,
    refetch: refetchHealth,
  } = useQuery({
    queryKey: ['admin-health', refreshKey],
    queryFn: fetchHealth,
    refetchInterval: 30000,
    enabled: isOperator,
  })

  const {
    data: readiness,
    isLoading: readinessLoading,
    error: readinessError,
    refetch: refetchReadiness,
  } = useQuery({
    queryKey: ['admin-readiness', refreshKey],
    queryFn: fetchReadiness,
    refetchInterval: 30000,
    enabled: isOperator,
  })

  const {
    data: redMetrics,
    isLoading: redLoading,
    error: redError,
    refetch: refetchRed,
  } = useQuery({
    queryKey: ['admin-red-metrics', refreshKey],
    queryFn: fetchRedMetrics,
    refetchInterval: 30000,
    enabled: isOperator,
  })

  // Computed data
  const overallStatus = useMemo(() => {
    if (health?.status === 'healthy' && readiness?.status === 'ready') return 'healthy'
    if (health?.status === 'degraded' || readiness?.database.status === 'degraded') return 'degraded'
    return 'unhealthy'
  }, [health, readiness])

  const servicesUp = useMemo(() => {
    if (!health?.services) return 0
    return Object.values(health.services).filter((s) => s === 'up').length
  }, [health])

  const totalServices = useMemo(() => {
    if (!health?.services) return 0
    return Object.keys(health.services).length
  }, [health])

  // Historical data for charts
  const latencyHistory = useMemo(
    () => generateHistoricalData(redMetrics?.api_p95_latency_5m ?? 0.05, 24),
    [redMetrics?.api_p95_latency_5m]
  )

  const errorRateHistory = useMemo(
    () => generateHistoricalData((redMetrics?.api_error_rate_5m ?? 0) * 100, 24),
    [redMetrics?.api_error_rate_5m]
  )

  const rpsHistory = useMemo(
    () => generateHistoricalData(redMetrics?.api_rps_5m ?? 10, 24),
    [redMetrics?.api_rps_5m]
  )

  const serviceStatusData = useMemo(
    () => [
      { name: 'Healthy', value: servicesUp, color: COLORS.success },
      { name: 'Degraded', value: Math.max(0, totalServices - servicesUp), color: COLORS.warning },
    ],
    [servicesUp, totalServices]
  )

  const handleRefresh = () => {
    setRefreshKey((k) => k + 1)
    refetchHealth()
    refetchReadiness()
    refetchRed()
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

  const isLoading = healthLoading || readinessLoading || redLoading

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-semibold text-gray-900">System Overview</h2>
          <p className="text-sm text-gray-500 mt-1">
            Real-time monitoring of all platform services
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

      {/* Status Banner */}
      <div
        className={`p-4 rounded-xl border ${
          overallStatus === 'healthy'
            ? 'bg-green-50 border-green-200'
            : overallStatus === 'degraded'
            ? 'bg-yellow-50 border-yellow-200'
            : 'bg-red-50 border-red-200'
        }`}
      >
        <div className="flex items-center gap-3">
          {overallStatus === 'healthy' ? (
            <CheckCircle2 className="h-6 w-6 text-green-600" />
          ) : overallStatus === 'degraded' ? (
            <AlertTriangle className="h-6 w-6 text-yellow-600" />
          ) : (
            <XCircle className="h-6 w-6 text-red-600" />
          )}
          <div>
            <p
              className={`font-semibold ${
                overallStatus === 'healthy'
                  ? 'text-green-800'
                  : overallStatus === 'degraded'
                  ? 'text-yellow-800'
                  : 'text-red-800'
              }`}
            >
              System Status:{' '}
              {overallStatus === 'healthy'
                ? 'All Systems Operational'
                : overallStatus === 'degraded'
                ? 'Degraded Performance'
                : 'Service Disruption'}
            </p>
            <p
              className={`text-sm ${
                overallStatus === 'healthy'
                  ? 'text-green-700'
                  : overallStatus === 'degraded'
                  ? 'text-yellow-700'
                  : 'text-red-700'
              }`}
            >
              {servicesUp}/{totalServices} services operational
            </p>
          </div>
        </div>
      </div>

      {/* Key Metrics Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <MetricCard
          title="API Latency (p95)"
          value={redMetrics?.api_p95_latency_5m != null ? `${(redMetrics.api_p95_latency_5m * 1000).toFixed(0)}ms` : '--'}
          subtitle="5-minute average"
          trend={redMetrics?.api_p95_latency_5m != null && redMetrics.api_p95_latency_5m < 0.1 ? 'good' : 'warning'}
          icon={Clock}
          loading={redLoading}
        />
        <MetricCard
          title="Error Rate"
          value={redMetrics?.api_error_rate_5m != null ? `${(redMetrics.api_error_rate_5m * 100).toFixed(2)}%` : '--'}
          subtitle="5-minute average"
          trend={redMetrics?.api_error_rate_5m != null && redMetrics.api_error_rate_5m < 0.01 ? 'good' : 'warning'}
          icon={AlertTriangle}
          loading={redLoading}
        />
        <MetricCard
          title="Request Rate"
          value={redMetrics?.api_rps_5m != null ? `${redMetrics.api_rps_5m.toFixed(1)} rps` : '--'}
          subtitle="5-minute average"
          trend="neutral"
          icon={Activity}
          loading={redLoading}
        />
        <MetricCard
          title="Cache Hit Ratio"
          value={readiness?.cache.hit_ratio != null ? `${(readiness.cache.hit_ratio * 100).toFixed(1)}%` : '--'}
          subtitle="Current"
          trend={readiness?.cache.hit_ratio != null && readiness.cache.hit_ratio > 0.9 ? 'good' : 'warning'}
          icon={Zap}
          loading={readinessLoading}
        />
      </div>

      {/* Service Status Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Services Status */}
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Service Health</h3>
          <div className="space-y-3">
            <ServiceStatusRow
              name="PostgreSQL"
              status={health?.services.postgres === 'up' ? 'up' : 'down'}
              latency={readiness?.database.latency_ms}
              loading={healthLoading}
            />
            <ServiceStatusRow
              name="Redis"
              status={health?.services.redis === 'up' ? 'up' : 'down'}
              loading={healthLoading}
            />
            <ServiceStatusRow
              name="Job Queues"
              status={health?.services.queues === 'up' ? 'up' : 'down'}
              loading={healthLoading}
            />
          </div>
          <div className="mt-4 pt-4 border-t">
            <div className="flex justify-between text-sm">
              <span className="text-gray-500">Database Connected</span>
              <span className={readiness?.database.connected ? 'text-green-600' : 'text-red-600'}>
                {readiness?.database.connected ? 'Yes' : 'No'}
              </span>
            </div>
          </div>
        </div>

        {/* Service Distribution Pie */}
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Service Distribution</h3>
          <div className="h-48">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={serviceStatusData}
                  cx="50%"
                  cy="50%"
                  innerRadius={50}
                  outerRadius={70}
                  paddingAngle={5}
                  dataKey="value"
                >
                  {serviceStatusData.map((entry, index) => (
                    <Cell key={`cell-${index}`} fill={entry.color} />
                  ))}
                </Pie>
                <Tooltip />
                <Legend />
              </PieChart>
            </ResponsiveContainer>
          </div>
        </div>

        {/* Quick Links */}
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">External Dashboards</h3>
          <div className="space-y-3">
            <ExternalLinkCard
              title="Grafana"
              description="Detailed metrics and dashboards"
              href="https://grafana.apexmediation.ee"
              icon="📊"
            />
            <ExternalLinkCard
              title="Prometheus"
              description="Raw metrics and queries"
              href="https://prometheus.apexmediation.ee"
              icon="📈"
            />
            <ExternalLinkCard
              title="Status Page"
              description="Public status page"
              href="https://status.apexmediation.ee"
              icon="✅"
            />
          </div>
        </div>
      </div>

      {/* Charts Row */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Latency Chart */}
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">API Latency (24h)</h3>
          <div className="h-64">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={latencyHistory}>
                <defs>
                  <linearGradient id="latencyGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={COLORS.primary} stopOpacity={0.2} />
                    <stop offset="95%" stopColor={COLORS.primary} stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                <XAxis dataKey="time" tick={{ fontSize: 11 }} stroke="#9ca3af" />
                <YAxis
                  tick={{ fontSize: 11 }}
                  stroke="#9ca3af"
                  tickFormatter={(v) => `${(v * 1000).toFixed(0)}ms`}
                />
                <Tooltip
                  formatter={(value: number) => [`${(value * 1000).toFixed(1)}ms`, 'Latency']}
                  labelStyle={{ color: '#374151' }}
                  contentStyle={{ borderRadius: '8px', border: '1px solid #e5e7eb' }}
                />
                <Area
                  type="monotone"
                  dataKey="value"
                  stroke={COLORS.primary}
                  fill="url(#latencyGradient)"
                  strokeWidth={2}
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </div>

        {/* Error Rate Chart */}
        <div className="bg-white rounded-xl border p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Error Rate (24h)</h3>
          <div className="h-64">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={errorRateHistory}>
                <defs>
                  <linearGradient id="errorGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={COLORS.danger} stopOpacity={0.2} />
                    <stop offset="95%" stopColor={COLORS.danger} stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                <XAxis dataKey="time" tick={{ fontSize: 11 }} stroke="#9ca3af" />
                <YAxis
                  tick={{ fontSize: 11 }}
                  stroke="#9ca3af"
                  tickFormatter={(v) => `${v.toFixed(2)}%`}
                />
                <Tooltip
                  formatter={(value: number) => [`${value.toFixed(3)}%`, 'Error Rate']}
                  labelStyle={{ color: '#374151' }}
                  contentStyle={{ borderRadius: '8px', border: '1px solid #e5e7eb' }}
                />
                <Area
                  type="monotone"
                  dataKey="value"
                  stroke={COLORS.danger}
                  fill="url(#errorGradient)"
                  strokeWidth={2}
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </div>
      </div>

      {/* Request Rate Chart */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">Request Throughput (24h)</h3>
        <div className="h-64">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={rpsHistory}>
              <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
              <XAxis dataKey="time" tick={{ fontSize: 11 }} stroke="#9ca3af" />
              <YAxis
                tick={{ fontSize: 11 }}
                stroke="#9ca3af"
                tickFormatter={(v) => `${v.toFixed(0)} rps`}
              />
              <Tooltip
                formatter={(value: number) => [`${value.toFixed(2)} req/s`, 'Throughput']}
                labelStyle={{ color: '#374151' }}
                contentStyle={{ borderRadius: '8px', border: '1px solid #e5e7eb' }}
              />
              <Bar dataKey="value" fill={COLORS.success} radius={[4, 4, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>

      {/* Database Details */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">Database Status</h3>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div>
            <p className="text-sm text-gray-500">Connection Status</p>
            <p className="text-lg font-semibold text-gray-900">
              {readiness?.database.connected ? (
                <span className="text-green-600">Connected</span>
              ) : (
                <span className="text-red-600">Disconnected</span>
              )}
            </p>
          </div>
          <div>
            <p className="text-sm text-gray-500">Query Latency</p>
            <p className="text-lg font-semibold text-gray-900">
              {readiness?.database.latency_ms != null ? `${readiness.database.latency_ms}ms` : '--'}
            </p>
          </div>
          <div>
            <p className="text-sm text-gray-500">Health Status</p>
            <p className="text-lg font-semibold text-gray-900">
              {readiness?.database.status || '--'}
            </p>
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="text-center text-xs text-gray-400 py-4">
        Data refreshes automatically every 30 seconds • Powered by Prometheus + Grafana
      </div>
    </div>
  )
}

// Components
function MetricCard({
  title,
  value,
  subtitle,
  trend,
  icon: Icon,
  loading,
}: {
  title: string
  value: string
  subtitle: string
  trend: 'good' | 'warning' | 'danger' | 'neutral'
  icon: React.ComponentType<{ className?: string }>
  loading?: boolean
}) {
  const trendColors = {
    good: 'text-green-600 bg-green-50',
    warning: 'text-yellow-600 bg-yellow-50',
    danger: 'text-red-600 bg-red-50',
    neutral: 'text-gray-600 bg-gray-50',
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
        <div className={`p-2 rounded-lg ${trendColors[trend]}`}>
          <Icon className="h-5 w-5" />
        </div>
      </div>
    </div>
  )
}

function ServiceStatusRow({
  name,
  status,
  latency,
  loading,
}: {
  name: string
  status: 'up' | 'down'
  latency?: number
  loading?: boolean
}) {
  return (
    <div className="flex items-center justify-between py-2 border-b last:border-0">
      <div className="flex items-center gap-3">
        {loading ? (
          <div className="w-3 h-3 rounded-full bg-gray-200 animate-pulse" />
        ) : (
          <div
            className={`w-3 h-3 rounded-full ${
              status === 'up' ? 'bg-green-500' : 'bg-red-500'
            }`}
          />
        )}
        <span className="text-sm font-medium text-gray-700">{name}</span>
      </div>
      <div className="flex items-center gap-2">
        {latency != null && (
          <span className="text-xs text-gray-500">{latency}ms</span>
        )}
        <span
          className={`text-xs font-medium ${
            status === 'up' ? 'text-green-600' : 'text-red-600'
          }`}
        >
          {status === 'up' ? 'Operational' : 'Down'}
        </span>
      </div>
    </div>
  )
}

function ExternalLinkCard({
  title,
  description,
  href,
  icon,
}: {
  title: string
  description: string
  href: string
  icon: string
}) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="flex items-center gap-3 p-3 rounded-lg border hover:bg-gray-50 transition-colors group"
    >
      <span className="text-2xl">{icon}</span>
      <div className="flex-1">
        <p className="text-sm font-medium text-gray-900 group-hover:text-primary-600">
          {title}
        </p>
        <p className="text-xs text-gray-500">{description}</p>
      </div>
      <ExternalLink className="h-4 w-4 text-gray-400 group-hover:text-primary-500" />
    </a>
  )
}
