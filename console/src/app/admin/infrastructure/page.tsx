'use client'

import { useState, useEffect, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'

// Types
interface HealthCheck {
  state: 'OK' | 'DEGRADED' | 'FAIL'
  checks: Record<string, {
    status: 'OK' | 'WARN' | 'FAIL'
    [key: string]: unknown
  }>
  timestamp: string
}

interface AutopilotStatus {
  state: 'OK' | 'DEGRADED' | 'FAIL'
  last_check: string
  checks: Record<string, unknown>
  metrics: Record<string, unknown>
  uptime: number
  version: string
}

interface DatabaseMetrics {
  connections: {
    active: number
    idle: number
    total: number
    max: number
    usage_pct: number
  }
  tables: Array<{
    schema: string
    name: string
    row_count: number
    size_bytes: number
    dead_tuples: number
  }>
  partitions: Array<{
    table: string
    partition_count: number
    oldest_partition: string
    newest_partition: string
  }>
  performance: {
    cache_hit_ratio: number
    transactions_per_sec: number
    tuples_returned_per_sec: number
  }
}

interface AutopilotJob {
  id: string
  job_type: string
  status: 'pending' | 'running' | 'completed' | 'failed' | 'skipped'
  scheduled_at: string
  started_at?: string
  completed_at?: string
  duration_ms?: number
  result?: Record<string, unknown>
  error?: string
}

// API base URL (from env or default)
const AUTOPILOT_API = process.env.NEXT_PUBLIC_AUTOPILOT_API_URL || 'http://localhost:8090'

// API client
async function autopilotFetch<T>(path: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`${AUTOPILOT_API}${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  })
  
  if (!response.ok) {
    throw new Error(`Autopilot API error: ${response.status}`)
  }
  
  return response.json()
}

// State indicator component
function StateIndicator({ state }: { state: 'OK' | 'DEGRADED' | 'FAIL' | 'WARN' }) {
  const colors = {
    OK: 'bg-green-500',
    WARN: 'bg-yellow-500',
    DEGRADED: 'bg-yellow-500',
    FAIL: 'bg-red-500',
  }
  
  return (
    <span className={`inline-flex h-3 w-3 rounded-full ${colors[state]}`} />
  )
}

// Format bytes to human readable
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`
}

// Format duration
function formatDuration(seconds: number): string {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  
  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${mins}m`
  return `${mins}m`
}

// Status Card Component
function StatusCard({ status }: { status: AutopilotStatus | undefined }) {
  if (!status) {
    return (
      <div className="rounded-xl border border-gray-200 bg-white p-6">
        <div className="animate-pulse space-y-4">
          <div className="h-4 bg-gray-200 rounded w-1/4"></div>
          <div className="h-8 bg-gray-200 rounded w-1/2"></div>
        </div>
      </div>
    )
  }
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-6">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-medium text-gray-500">Autopilot Status</p>
          <div className="mt-2 flex items-center gap-3">
            <StateIndicator state={status.state} />
            <span className="text-2xl font-semibold text-gray-900">{status.state}</span>
          </div>
        </div>
        <div className="text-right">
          <p className="text-sm text-gray-500">Uptime</p>
          <p className="text-lg font-medium text-gray-900">{formatDuration(status.uptime)}</p>
        </div>
      </div>
      <div className="mt-4 flex items-center gap-4 text-sm text-gray-500">
        <span>Version: {status.version}</span>
        <span>Last check: {new Date(status.last_check).toLocaleTimeString()}</span>
      </div>
    </div>
  )
}

// Connections Card Component
function ConnectionsCard({ metrics }: { metrics: DatabaseMetrics | undefined }) {
  if (!metrics) {
    return (
      <div className="rounded-xl border border-gray-200 bg-white p-6">
        <div className="animate-pulse space-y-4">
          <div className="h-4 bg-gray-200 rounded w-1/4"></div>
          <div className="h-8 bg-gray-200 rounded w-full"></div>
        </div>
      </div>
    )
  }
  
  const { connections } = metrics
  const usageColor = connections.usage_pct > 80 
    ? 'bg-red-500' 
    : connections.usage_pct > 60 
      ? 'bg-yellow-500' 
      : 'bg-green-500'
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-6">
      <p className="text-sm font-medium text-gray-500">Database Connections</p>
      <div className="mt-4">
        <div className="flex justify-between text-sm mb-2">
          <span className="text-gray-600">{connections.total} / {connections.max}</span>
          <span className="font-medium">{connections.usage_pct.toFixed(1)}%</span>
        </div>
        <div className="h-2 bg-gray-200 rounded-full overflow-hidden">
          <div 
            className={`h-full ${usageColor} transition-all duration-300`}
            style={{ width: `${Math.min(connections.usage_pct, 100)}%` }}
          />
        </div>
      </div>
      <div className="mt-4 grid grid-cols-2 gap-4 text-sm">
        <div>
          <p className="text-gray-500">Active</p>
          <p className="font-medium text-gray-900">{connections.active}</p>
        </div>
        <div>
          <p className="text-gray-500">Idle</p>
          <p className="font-medium text-gray-900">{connections.idle}</p>
        </div>
      </div>
    </div>
  )
}

// Performance Card Component
function PerformanceCard({ metrics }: { metrics: DatabaseMetrics | undefined }) {
  if (!metrics) {
    return (
      <div className="rounded-xl border border-gray-200 bg-white p-6">
        <div className="animate-pulse space-y-4">
          <div className="h-4 bg-gray-200 rounded w-1/4"></div>
          <div className="h-8 bg-gray-200 rounded w-full"></div>
        </div>
      </div>
    )
  }
  
  const cacheRatio = metrics.performance.cache_hit_ratio
  const cacheColor = cacheRatio < 80 
    ? 'text-red-600' 
    : cacheRatio < 90 
      ? 'text-yellow-600' 
      : 'text-green-600'
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-6">
      <p className="text-sm font-medium text-gray-500">Cache Performance</p>
      <div className="mt-4">
        <p className={`text-3xl font-bold ${cacheColor}`}>
          {cacheRatio.toFixed(1)}%
        </p>
        <p className="text-sm text-gray-500 mt-1">Cache Hit Ratio</p>
      </div>
    </div>
  )
}

// Tables Table Component
function TablesTable({ tables }: { tables: DatabaseMetrics['tables'] | undefined }) {
  if (!tables || tables.length === 0) {
    return (
      <div className="rounded-xl border border-gray-200 bg-white p-6">
        <p className="text-sm font-medium text-gray-500 mb-4">Top Tables by Size</p>
        <p className="text-gray-400 text-sm">No table data available</p>
      </div>
    )
  }
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
      <div className="px-6 py-4 border-b border-gray-200">
        <p className="text-sm font-medium text-gray-900">Top Tables by Size</p>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full">
          <thead className="bg-gray-50">
            <tr>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase">Schema</th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase">Table</th>
              <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase">Rows</th>
              <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase">Size</th>
              <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase">Dead Tuples</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200">
            {tables.slice(0, 10).map((table, idx) => (
              <tr key={idx} className="hover:bg-gray-50">
                <td className="px-6 py-3 text-sm text-gray-600">{table.schema}</td>
                <td className="px-6 py-3 text-sm font-medium text-gray-900">{table.name}</td>
                <td className="px-6 py-3 text-sm text-gray-600 text-right">{table.row_count.toLocaleString()}</td>
                <td className="px-6 py-3 text-sm text-gray-600 text-right">{formatBytes(table.size_bytes)}</td>
                <td className="px-6 py-3 text-sm text-right">
                  <span className={table.dead_tuples > table.row_count * 0.1 ? 'text-yellow-600' : 'text-gray-600'}>
                    {table.dead_tuples.toLocaleString()}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

// Partitions Card Component
function PartitionsCard({ partitions }: { partitions: DatabaseMetrics['partitions'] | undefined }) {
  if (!partitions || partitions.length === 0) {
    return (
      <div className="rounded-xl border border-gray-200 bg-white p-6">
        <p className="text-sm font-medium text-gray-500 mb-4">Partitions</p>
        <p className="text-gray-400 text-sm">No partition data available</p>
      </div>
    )
  }
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
      <div className="px-6 py-4 border-b border-gray-200">
        <p className="text-sm font-medium text-gray-900">Partitions</p>
      </div>
      <div className="divide-y divide-gray-200">
        {partitions.map((partition, idx) => (
          <div key={idx} className="px-6 py-4">
            <div className="flex justify-between items-center">
              <p className="font-medium text-gray-900">{partition.table}</p>
              <span className="text-sm text-gray-500">{partition.partition_count} partitions</span>
            </div>
            <div className="mt-1 text-xs text-gray-500">
              {partition.oldest_partition} → {partition.newest_partition}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

// Jobs Table Component
function JobsTable({ jobs }: { jobs: AutopilotJob[] | undefined }) {
  if (!jobs || jobs.length === 0) {
    return (
      <div className="rounded-xl border border-gray-200 bg-white p-6">
        <p className="text-sm font-medium text-gray-500 mb-4">Recent Jobs</p>
        <p className="text-gray-400 text-sm">No jobs found</p>
      </div>
    )
  }
  
  const statusColors = {
    pending: 'bg-gray-100 text-gray-700',
    running: 'bg-blue-100 text-blue-700',
    completed: 'bg-green-100 text-green-700',
    failed: 'bg-red-100 text-red-700',
    skipped: 'bg-yellow-100 text-yellow-700',
  }
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
      <div className="px-6 py-4 border-b border-gray-200">
        <p className="text-sm font-medium text-gray-900">Recent Jobs</p>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full">
          <thead className="bg-gray-50">
            <tr>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase">Job Type</th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase">Status</th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase">Scheduled</th>
              <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase">Duration</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200">
            {jobs.slice(0, 20).map((job) => (
              <tr key={job.id} className="hover:bg-gray-50">
                <td className="px-6 py-3 text-sm font-medium text-gray-900">{job.job_type}</td>
                <td className="px-6 py-3">
                  <span className={`inline-flex px-2 py-1 rounded-full text-xs font-medium ${statusColors[job.status]}`}>
                    {job.status}
                  </span>
                </td>
                <td className="px-6 py-3 text-sm text-gray-600">
                  {new Date(job.scheduled_at).toLocaleString()}
                </td>
                <td className="px-6 py-3 text-sm text-gray-600 text-right">
                  {job.duration_ms ? `${job.duration_ms}ms` : '-'}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

// Actions Panel Component
function ActionsPanel({ 
  onTriggerJob, 
  isTriggering 
}: { 
  onTriggerJob: (jobType: string) => void
  isTriggering: boolean 
}) {
  const jobs = [
    { type: 'health_check', label: 'Run Health Check', description: 'Check database health status' },
    { type: 'ensure_partitions', label: 'Ensure Partitions', description: 'Create future partitions' },
    { type: 'retention_cleanup', label: 'Retention Cleanup', description: 'Remove old partitions' },
    { type: 'hourly_aggregation', label: 'Hourly Aggregation', description: 'Compute hourly metrics' },
    { type: 'daily_aggregation', label: 'Daily Aggregation', description: 'Compute daily metrics' },
  ]
  
  const actions = [
    { action: 'vacuum_analyze', label: 'VACUUM ANALYZE', description: 'Clean up dead tuples', danger: false },
    { action: 'kill_idle_connections', label: 'Kill Idle Connections', description: 'Terminate idle connections > 1h', danger: true },
  ]
  
  return (
    <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
      <div className="px-6 py-4 border-b border-gray-200">
        <p className="text-sm font-medium text-gray-900">Manual Actions</p>
        <p className="text-xs text-gray-500 mt-1">Trigger jobs or maintenance actions manually</p>
      </div>
      <div className="p-6 space-y-6">
        <div>
          <p className="text-xs font-medium text-gray-500 uppercase mb-3">Scheduled Jobs</p>
          <div className="grid grid-cols-2 gap-3">
            {jobs.map((job) => (
              <button
                key={job.type}
                onClick={() => onTriggerJob(job.type)}
                disabled={isTriggering}
                className="flex flex-col items-start p-3 rounded-lg border border-gray-200 hover:border-primary-300 hover:bg-primary-50 transition-colors disabled:opacity-50"
              >
                <span className="text-sm font-medium text-gray-900">{job.label}</span>
                <span className="text-xs text-gray-500 mt-0.5">{job.description}</span>
              </button>
            ))}
          </div>
        </div>
        
        <div>
          <p className="text-xs font-medium text-gray-500 uppercase mb-3">Maintenance Actions</p>
          <div className="grid grid-cols-2 gap-3">
            {actions.map((action) => (
              <button
                key={action.action}
                onClick={() => onTriggerJob(`action:${action.action}`)}
                disabled={isTriggering}
                className={`flex flex-col items-start p-3 rounded-lg border transition-colors disabled:opacity-50 ${
                  action.danger 
                    ? 'border-red-200 hover:border-red-300 hover:bg-red-50' 
                    : 'border-gray-200 hover:border-primary-300 hover:bg-primary-50'
                }`}
              >
                <span className={`text-sm font-medium ${action.danger ? 'text-red-700' : 'text-gray-900'}`}>
                  {action.label}
                </span>
                <span className="text-xs text-gray-500 mt-0.5">{action.description}</span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

// Main Page Component
export default function InfrastructurePage() {
  const queryClient = useQueryClient()
  const [lastRefresh, setLastRefresh] = useState<Date>(new Date())
  
  // Status query
  const { data: status, error: statusError, isLoading: statusLoading } = useQuery({
    queryKey: ['autopilot-status'],
    queryFn: () => autopilotFetch<AutopilotStatus>('/status'),
    refetchInterval: 10000, // Refresh every 10 seconds
  })
  
  // Metrics query
  const { data: metrics, error: metricsError, isLoading: metricsLoading } = useQuery({
    queryKey: ['autopilot-metrics'],
    queryFn: () => autopilotFetch<DatabaseMetrics>('/metrics'),
    refetchInterval: 30000, // Refresh every 30 seconds
  })
  
  // Jobs query
  const { data: jobsData } = useQuery({
    queryKey: ['autopilot-jobs'],
    queryFn: () => autopilotFetch<{ jobs: AutopilotJob[] }>('/jobs'),
    refetchInterval: 30000,
  })
  
  // Trigger job mutation
  const triggerJobMutation = useMutation({
    mutationFn: async (jobType: string) => {
      if (jobType.startsWith('action:')) {
        const action = jobType.replace('action:', '')
        return autopilotFetch(`/actions/${action}`, { method: 'POST' })
      }
      return autopilotFetch(`/jobs/trigger/${jobType}`, { method: 'POST' })
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['autopilot-status'] })
      queryClient.invalidateQueries({ queryKey: ['autopilot-metrics'] })
      queryClient.invalidateQueries({ queryKey: ['autopilot-jobs'] })
    },
  })
  
  // Refresh handler
  const handleRefresh = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ['autopilot-status'] })
    queryClient.invalidateQueries({ queryKey: ['autopilot-metrics'] })
    queryClient.invalidateQueries({ queryKey: ['autopilot-jobs'] })
    setLastRefresh(new Date())
  }, [queryClient])
  
  // Handle connection errors gracefully
  const isConnected = !statusError && !metricsError
  
  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-gray-900">Infrastructure Control Panel</h2>
          <p className="text-sm text-gray-500 mt-1">
            Database autopilot and monitoring
          </p>
        </div>
        <div className="flex items-center gap-4">
          <span className="text-xs text-gray-400">
            Last refresh: {lastRefresh.toLocaleTimeString()}
          </span>
          <button
            onClick={handleRefresh}
            className="px-3 py-1.5 text-sm font-medium text-primary-700 bg-primary-50 rounded-lg hover:bg-primary-100 transition-colors"
          >
            Refresh
          </button>
        </div>
      </div>
      
      {/* Connection Error Banner */}
      {!isConnected && (
        <div className="rounded-xl border border-red-200 bg-red-50 p-4">
          <div className="flex items-center gap-3">
            <StateIndicator state="FAIL" />
            <div>
              <p className="text-sm font-medium text-red-800">Connection Error</p>
              <p className="text-xs text-red-600 mt-0.5">
                Unable to connect to Autopilot service at {AUTOPILOT_API}
              </p>
            </div>
          </div>
        </div>
      )}
      
      {/* Status Overview */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <StatusCard status={status} />
        <ConnectionsCard metrics={metrics} />
        <PerformanceCard metrics={metrics} />
      </div>
      
      {/* Main Content Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Left Column - Tables & Partitions */}
        <div className="lg:col-span-2 space-y-6">
          <TablesTable tables={metrics?.tables} />
          <JobsTable jobs={jobsData?.jobs} />
        </div>
        
        {/* Right Column - Actions & Partitions */}
        <div className="space-y-6">
          <PartitionsCard partitions={metrics?.partitions} />
          <ActionsPanel 
            onTriggerJob={(jobType) => triggerJobMutation.mutate(jobType)} 
            isTriggering={triggerJobMutation.isPending}
          />
        </div>
      </div>
    </div>
  )
}
